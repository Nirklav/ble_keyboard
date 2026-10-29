//! Firmware update over the config protocol, see [`crate::hid`] for the message flow.
//!
//! The app sends the app image (`espflash save-image` output, see [`crate::image`]) raw deflate
//! compressed with [`compress`]: about half the size, so half the transfer time. The keyboard
//! decompresses it on the fly with [`FirmwareReceiver`] and writes it to the update slot in blocks of
//! [`BLOCK_SIZE`]; a block is never kept longer than that, the image doesn't fit the RAM.
//!
//! The new firmware confirms itself on the first config request after the restart; the app reconnects
//! and asks for [`Request::GetVersions`](crate::hid::Request::GetVersions) for that. Without a request for
//! [`ROLLBACK_TIMEOUT`] the keyboard goes back to the old firmware.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::crc::crc32_update;
use crate::hid::ErrorCode;
use crate::image;

/// Decompressed bytes per write to the flash: a flash sector.
pub const BLOCK_SIZE: usize = 4096;

/// A new firmware that gets no config request this long after its start goes back to the old one: time for
/// the host to reconnect after the restart.
pub const ROLLBACK_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(5 * 60);

/// `image` compressed for [`crate::hid::Request::FirmwareBegin`].
pub fn compress(image: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(image, 10)
}

/// Keyboard side of the firmware update: checks and decompresses the chunks.
///
/// The image goes to `write` (the flash) in order, in blocks of [`BLOCK_SIZE`] but the last one; the
/// first block is checked to be a firmware of the receiver's board ([`image::AppInfo::is_for`]) before it's
/// written. Any error aborts the transfer, what was written is garbage then.
pub struct FirmwareReceiver {
    /// Project name of the running firmware, see [`crate::image`].
    board: &'static str,
    transfer: Option<Transfer>,
}

struct Transfer {
    board: &'static str,
    /// Compressed length and CRC-32, announced.
    len: usize,
    crc32: u32,
    /// Decompressed length, announced.
    image_len: usize,
    /// Compressed bytes received and their CRC-32.
    received: usize,
    crc: u32,
    /// Boxed by miniz: the 32 KiB window and the decompressor state.
    inflate: Box<InflateState>,
    /// Decompressed, not written yet: `block[..filled]`.
    block: Vec<u8>,
    filled: usize,
    /// Decompressed bytes written.
    written: usize,
    /// The deflate stream is complete.
    ended: bool,
}

impl FirmwareReceiver {
    /// Receiver for a firmware of `board`.
    pub fn new(board: &'static str) -> Self {
        Self { board, transfer: None }
    }

    pub fn is_active(&self) -> bool {
        self.transfer.is_some()
    }

    /// Drops the current transfer.
    pub fn abort(&mut self) {
        self.transfer = None;
    }

    /// [`crate::hid::Request::FirmwareBegin`]: starts a new transfer, dropping the current one.
    /// `max_image_len` is the size of the update slot.
    pub fn begin(&mut self, len: u32, crc32: u32, image_len: u32, max_image_len: usize) -> Result<(), ErrorCode> {
        self.transfer = None;
        let (len, image_len) = (len as usize, image_len as usize);
        if len == 0 || image_len < image::HEADER_LEN {
            return Err(ErrorCode::BadRequest);
        }
        if image_len > max_image_len {
            return Err(ErrorCode::TooLarge);
        }
        self.transfer = Some(Transfer {
            board: self.board,
            len,
            crc32,
            image_len,
            received: 0,
            crc: 0,
            inflate: InflateState::new_boxed(DataFormat::Raw),
            block: vec![0; BLOCK_SIZE],
            filled: 0,
            written: 0,
            ended: false,
        });
        Ok(())
    }

    /// [`crate::hid::Request::FirmwareChunk`]: decompresses `data`, `offset` must equal the number of
    /// bytes received so far. Full blocks go to `write`.
    pub fn chunk(
        &mut self,
        offset: u32,
        data: &[u8],
        write: impl FnMut(&[u8]) -> Result<(), ErrorCode>,
    ) -> Result<(), ErrorCode> {
        let result = self.try_chunk(offset, data, write);
        if result.is_err() {
            self.transfer = None;
        }
        result
    }

    fn try_chunk(
        &mut self,
        offset: u32,
        data: &[u8],
        write: impl FnMut(&[u8]) -> Result<(), ErrorCode>,
    ) -> Result<(), ErrorCode> {
        let transfer = self.transfer.as_mut().ok_or(ErrorCode::OutOfOrder)?;
        if offset as usize != transfer.received {
            return Err(ErrorCode::OutOfOrder);
        }
        if transfer.received + data.len() > transfer.len {
            return Err(ErrorCode::TooLarge);
        }
        transfer.received += data.len();
        transfer.crc = crc32_update(transfer.crc, data);
        transfer.inflate(data, write)
    }

    /// [`crate::hid::Request::FirmwareCommit`]: checks length and CRC, writes the last block.
    /// Ends the transfer whatever the result is; on success the whole image is written.
    pub fn commit(&mut self, mut write: impl FnMut(&[u8]) -> Result<(), ErrorCode>) -> Result<(), ErrorCode> {
        let mut transfer = self.transfer.take().ok_or(ErrorCode::OutOfOrder)?;
        if transfer.received != transfer.len {
            return Err(ErrorCode::OutOfOrder);
        }
        if transfer.crc != transfer.crc32 {
            return Err(ErrorCode::CrcMismatch);
        }
        if !transfer.ended || transfer.written + transfer.filled != transfer.image_len {
            return Err(ErrorCode::InvalidImage);
        }
        transfer.flush(&mut write)
    }
}

impl Transfer {
    fn inflate(&mut self, mut input: &[u8], mut write: impl FnMut(&[u8]) -> Result<(), ErrorCode>) -> Result<(), ErrorCode> {
        if self.ended {
            return if input.is_empty() { Ok(()) } else { Err(ErrorCode::InvalidImage) };
        }
        loop {
            if self.filled == BLOCK_SIZE {
                self.flush(&mut write)?;
            }
            let result = inflate(&mut self.inflate, input, &mut self.block[self.filled..], MZFlush::None);
            input = &input[result.bytes_consumed..];
            self.filled += result.bytes_written;
            if self.written + self.filled > self.image_len {
                return Err(ErrorCode::InvalidImage);
            }
            match result.status {
                Ok(MZStatus::StreamEnd) => {
                    self.ended = true;
                    return if input.is_empty() { Ok(()) } else { Err(ErrorCode::InvalidImage) };
                },
                // `Buf`: no progress without more input, or more room in the block.
                Ok(_) | Err(MZError::Buf) => {},
                Err(_) => return Err(ErrorCode::InvalidImage),
            }
            // A full block may hold back more output: it's taken on the next turn, input or not.
            if input.is_empty() && self.filled < BLOCK_SIZE {
                return Ok(());
            }
        }
    }

    fn flush(&mut self, write: &mut impl FnMut(&[u8]) -> Result<(), ErrorCode>) -> Result<(), ErrorCode> {
        // The image is a whole number of blocks.
        if self.filled == 0 {
            return Ok(());
        }
        let block = &self.block[..self.filled];
        if self.written == 0 {
            let info = image::app_info(block).map_err(|_| ErrorCode::InvalidImage)?;
            if !info.is_for(self.board) {
                return Err(ErrorCode::InvalidImage);
            }
        }
        write(block)?;
        self.written += self.filled;
        self.filled = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use super::*;
    use crate::crc::crc32;
    use crate::hid::{chunks, CHUNK_SIZE};
    use crate::image::tests::sample;

    const SLOT: usize = 64 * 1024;

    /// Keyboard image of `len` bytes, compressible like code (repeats with variations).
    fn image(len: usize) -> Vec<u8> {
        let body: Vec<u8> = (0..len - image::HEADER_LEN).map(|i| ((i * 7) ^ (i >> 5)) as u8).collect();
        sample(&body)
    }

    /// Sends `compressed`, returns the written blocks.
    fn send(
        receiver: &mut FirmwareReceiver,
        compressed: &[u8],
        image_len: usize,
        max_data: usize,
    ) -> Result<Vec<Vec<u8>>, ErrorCode> {
        let mut blocks = Vec::new();
        receiver.begin(compressed.len() as u32, crc32(compressed), image_len as u32, SLOT)?;
        for (offset, chunk) in chunks(compressed, max_data) {
            receiver.chunk(offset, chunk, |block| Ok(blocks.push(block.to_vec())))?;
        }
        receiver.commit(|block| Ok(blocks.push(block.to_vec())))?;
        Ok(blocks)
    }

    #[test]
    fn happy_path() {
        let mut receiver = FirmwareReceiver::new(image::KEYBOARD_PROJECT);
        for len in [image::HEADER_LEN, 1000, BLOCK_SIZE, 3 * BLOCK_SIZE + 17, 40_000] {
            let image = image(len);
            let compressed = compress(&image);
            for max_data in [1, 7, CHUNK_SIZE] {
                let blocks = send(&mut receiver, &compressed, image.len(), max_data).unwrap();
                assert_eq!(blocks.concat(), image, "len {len}, chunks of {max_data}");
                let (last, full) = blocks.split_last().unwrap();
                assert!(full.iter().all(|block| block.len() == BLOCK_SIZE));
                assert!(!last.is_empty() && last.len() <= BLOCK_SIZE);
                assert!(!receiver.is_active());
            }
        }
    }

    #[test]
    fn compresses_code_like_data() {
        let image = image(40_000);
        assert!(compress(&image).len() < image.len() / 2);
    }

    #[test]
    fn rejects_other_images() {
        let mut receiver = FirmwareReceiver::new(image::KEYBOARD_PROJECT);

        let mut joystick = image(10_000);
        joystick[0x50..0x59].copy_from_slice(b"joystick\0");
        assert_eq!(send(&mut receiver, &compress(&joystick), joystick.len(), CHUNK_SIZE), Err(ErrorCode::InvalidImage));
        assert!(!receiver.is_active());

        let mut elf = image(10_000);
        elf[..4].copy_from_slice(b"\x7fELF");
        assert_eq!(send(&mut receiver, &compress(&elf), elf.len(), CHUNK_SIZE), Err(ErrorCode::InvalidImage));

        let garbage = [0x55u8; 1000];
        assert_eq!(send(&mut receiver, &garbage, 2000, CHUNK_SIZE), Err(ErrorCode::InvalidImage));
    }

    #[test]
    fn checks_lengths() {
        let mut receiver = FirmwareReceiver::new(image::KEYBOARD_PROJECT);
        let image = image(10_000);
        let compressed = compress(&image);

        assert_eq!(send(&mut receiver, &compressed, image.len() - 1, CHUNK_SIZE), Err(ErrorCode::InvalidImage));
        assert_eq!(send(&mut receiver, &compressed, image.len() + 1, CHUNK_SIZE), Err(ErrorCode::InvalidImage));
        assert_eq!(receiver.begin(10, 0, SLOT as u32 + 1, SLOT), Err(ErrorCode::TooLarge));
        assert_eq!(receiver.begin(0, 0, 1000, SLOT), Err(ErrorCode::BadRequest));
        assert_eq!(receiver.begin(10, 0, 10, SLOT), Err(ErrorCode::BadRequest));

        // Data past the end of the deflate stream.
        let mut longer = compressed.clone();
        longer.push(0);
        assert_eq!(send(&mut receiver, &longer, image.len(), CHUNK_SIZE), Err(ErrorCode::InvalidImage));

        // The stream is cut short: all announced bytes arrived, but it didn't end.
        let short = &compressed[..compressed.len() / 2];
        assert_eq!(send(&mut receiver, short, image.len(), CHUNK_SIZE), Err(ErrorCode::InvalidImage));
    }

    #[test]
    fn out_of_order_and_crc() {
        let mut receiver = FirmwareReceiver::new(image::KEYBOARD_PROJECT);
        let image = image(10_000);
        let compressed = compress(&image);
        let ignore = |_: &[u8]| Ok(());

        assert_eq!(receiver.chunk(0, &compressed, ignore), Err(ErrorCode::OutOfOrder));
        assert_eq!(receiver.commit(ignore), Err(ErrorCode::OutOfOrder));

        receiver.begin(compressed.len() as u32, crc32(&compressed), image.len() as u32, SLOT).unwrap();
        receiver.chunk(0, &compressed[..10], ignore).unwrap();
        assert_eq!(receiver.chunk(20, &compressed[20..30], ignore), Err(ErrorCode::OutOfOrder));
        assert!(!receiver.is_active());

        receiver.begin(compressed.len() as u32, crc32(&compressed), image.len() as u32, SLOT).unwrap();
        receiver.chunk(0, &compressed[..10], ignore).unwrap();
        assert_eq!(receiver.commit(ignore), Err(ErrorCode::OutOfOrder));

        receiver.begin(compressed.len() as u32, crc32(&compressed) ^ 1, image.len() as u32, SLOT).unwrap();
        receiver.chunk(0, &compressed, ignore).unwrap();
        assert_eq!(receiver.commit(ignore), Err(ErrorCode::CrcMismatch));
    }

    #[test]
    fn write_errors_abort() {
        let mut receiver = FirmwareReceiver::new(image::KEYBOARD_PROJECT);
        let image = image(3 * BLOCK_SIZE);
        let compressed = compress(&image);
        receiver.begin(compressed.len() as u32, crc32(&compressed), image.len() as u32, SLOT).unwrap();
        assert_eq!(receiver.chunk(0, &compressed, |_| Err(ErrorCode::Storage)), Err(ErrorCode::Storage));
        assert!(!receiver.is_active());
    }
}
