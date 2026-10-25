//! BLE GATT config service: the desktop app (GATT client) reads and writes the
//! keyboard [`Config`](crate::config::Config) over the HID connection that the OS already has.
//!
//! The service ([`SERVICE_UUID`]) has three characteristics, all need an encrypted
//! (bonded) link:
//!
//! | Characteristic        | Properties                | Content                                    |
//! | --------------------- | ------------------------- | ------------------------------------------ |
//! | [`INFO_UUID`]         | read                      | postcard encoded [`Info`]                  |
//! | [`CONTROL_UUID`]      | write with response       | postcard encoded [`Request`], one per write |
//! | [`RESPONSE_UUID`]     | notify                    | postcard encoded [`Response`], one per notification |
//!
//! Every [`Request`] is answered by exactly one [`Response`] notification, the client
//! subscribes to [`RESPONSE_UUID`] first and sends the next request only after the
//! response to the previous one arrived. A write and a notification carry at most
//! `MTU - 3` bytes, so configs are transferred in chunks of [`chunk_payload_size`] bytes.
//! All numbers in the messages are offsets/lengths in bytes of the postcard encoded config.
//!
//! Write flow (`bytes = config.to_bytes()`):
//! 1. [`Request::WriteBegin`] with `len = bytes.len()` and `crc32 = crc32(&bytes)` → [`Response::Ok`].
//!    Starts a new transfer and drops an unfinished one.
//! 2. [`Request::WriteChunk`] for every chunk, in order, `offset` is the chunk start → [`Response::Ok`].
//! 3. [`Request::WriteCommit`] → the keyboard checks length, CRC, decodes and validates the
//!    config, applies it (once no key is held) and saves it to flash, then answers [`Response::Ok`].
//!
//! Read flow: [`Request::ReadConfig`] with `offset = 0` → [`Response::ConfigChunk`] with the total
//! length and the first chunk; repeat with `offset += data.len()` until `offset == total_len`.
//! The keyboard encodes its current config on every request, so `total_len` is checked to be the
//! same in all chunks.
//!
//! Any failed request is answered with [`Response::Error`]; a failed write step aborts the
//! transfer, the client starts again from [`Request::WriteBegin`].
//! [`Request::ResetToDefault`] replaces the config with the factory one → [`Response::Ok`].
//!
//! Session: while the app is connected it sends [`Request::Ping`] every [`PING_INTERVAL`] and
//! [`Request::End`] when it leaves. Until then (or [`SESSION_TIMEOUT`] after the last request,
//! or a disconnect) the keyboard doesn't enter deep sleep. Any request starts a session, there's
//! no separate "hello".
//!
//! Compatibility: new requests, responses and error codes are only appended;
//! [`PROTOCOL_VERSION`] is bumped on any other change.

/// Version of this protocol, reported in [`Info::protocol_version`].
pub const PROTOCOL_VERSION: u16 = 1;

/// How often the app sends [`Request::Ping`] while it's connected.
pub const PING_INTERVAL: core::time::Duration = core::time::Duration::from_secs(5);

/// The keyboard ends a session this long after the last request: a few missed pings.
pub const SESSION_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(20);

/// Config service UUID.
pub const SERVICE_UUID: u128 = 0x0da8123c_a800_4512_893d_e4857a5ae081;
/// [`SERVICE_UUID`] as a string, for `uuid128!`.
pub const SERVICE_UUID_STR: &str = "0da8123c-a800-4512-893d-e4857a5ae081";

/// Info characteristic UUID (read).
pub const INFO_UUID: u128 = 0xa6addb6d_7f16_46f4_87d4_73fbdfc3dcd8;
/// [`INFO_UUID`] as a string.
pub const INFO_UUID_STR: &str = "a6addb6d-7f16-46f4-87d4-73fbdfc3dcd8";

/// Control characteristic UUID (write with response).
pub const CONTROL_UUID: u128 = 0xaf1ee8b8_de69_41da_9f7b_d96ea59095af;
/// [`CONTROL_UUID`] as a string.
pub const CONTROL_UUID_STR: &str = "af1ee8b8-de69-41da-9f7b-d96ea59095af";

/// Response characteristic UUID (notify).
pub const RESPONSE_UUID: u128 = 0xac4b8720_b9cf_49a8_9241_30f90a5ccb7b;
/// [`RESPONSE_UUID`] as a string.
pub const RESPONSE_UUID_STR: &str = "ac4b8720-b9cf-49a8-9241-30f90a5ccb7b";

/// Max length of an encoded config the keyboard accepts.
pub const MAX_CONFIG_SIZE: usize = 8 * 1024;

/// ATT header of a write / notification (opcode + handle).
pub const ATT_HEADER: usize = 3;

/// Default ATT MTU before the exchange.
pub const DEFAULT_MTU: usize = 23;

/// Max postcard overhead of [`Request::WriteChunk`] / [`Response::ConfigChunk`] around the data:
/// variant tag (1) + two `u32` varints (5 + 5) + data length varint (2, for data < 16 KiB).
pub const CHUNK_OVERHEAD: usize = 1 + 5 + 5 + 2;

/// Max number of config bytes per [`Request::WriteChunk`] / [`Response::ConfigChunk`],
/// so that the encoded message fits a write / notification with this ATT `mtu`.
pub const fn chunk_payload_size(mtu: usize) -> usize {
    mtu.saturating_sub(ATT_HEADER + CHUNK_OVERHEAD)
}

/// Splits `bytes` into `(offset, chunk)` of at most `max_data` bytes (see [`chunk_payload_size`]).
pub fn chunks(bytes: &[u8], max_data: usize) -> impl Iterator<Item = (u32, &[u8])> {
    let max_data = max_data.max(1);
    bytes.chunks(max_data).enumerate().map(move |(i, chunk)| ((i * max_data) as u32, chunk))
}

#[cfg(feature = "alloc")]
pub use self::messages::*;

#[cfg(feature = "alloc")]
mod messages {
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::fmt;

    use serde::{Deserialize, Serialize};

    use super::MAX_CONFIG_SIZE;
    use crate::config::Config;
    use crate::crc::crc32;

    /// Content of the info characteristic.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Info {
        /// [`PROTOCOL_VERSION`](super::PROTOCOL_VERSION) of the firmware.
        pub protocol_version: u16,
        /// [`CONFIG_VERSION`](crate::config::CONFIG_VERSION) of the firmware.
        pub config_version: u16,
        /// Number of key columns.
        pub width: u8,
        /// Number of key rows.
        pub height: u8,
        /// [`MAX_LAYOUTS`](crate::config::MAX_LAYOUTS) of the firmware.
        pub max_layouts: u8,
        /// [`MAX_COMBOS`](crate::config::MAX_COMBOS) of the firmware.
        pub max_combos: u8,
        /// Firmware crate version.
        pub firmware_version: String,
    }

    /// Command written to the control characteristic.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Request {
        /// → [`Response::Info`].
        GetInfo,
        /// → [`Response::ConfigChunk`] starting at `offset`.
        ReadConfig { offset: u32 },
        /// Starts a config write of `len` bytes with CRC-32 `crc32` → [`Response::Ok`].
        WriteBegin { len: u32, crc32: u32 },
        /// Next chunk of the config, `offset` must be the number of bytes written so far → [`Response::Ok`].
        WriteChunk { offset: u32, data: Vec<u8> },
        /// Checks, applies and saves the written config → [`Response::Ok`].
        WriteCommit,
        /// Replaces the config with the factory one → [`Response::Ok`].
        ResetToDefault,
        /// Keeps the session alive, see [`PING_INTERVAL`](super::PING_INTERVAL) → [`Response::Ok`].
        Ping,
        /// Ends the session, the keyboard may sleep again → [`Response::Ok`].
        End,
    }

    /// Notification of the response characteristic.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Response {
        Info(Info),
        /// `data` of the encoded config at `offset`, empty once `offset >= total_len`.
        ConfigChunk { total_len: u32, offset: u32, data: Vec<u8> },
        Ok,
        Error(ErrorCode),
    }

    /// Why a request failed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum ErrorCode {
        /// Request can't be decoded.
        BadRequest,
        /// Write step without [`Request::WriteBegin`], chunk at a wrong offset or commit before all bytes arrived.
        OutOfOrder,
        /// Config is larger than [`MAX_CONFIG_SIZE`] or a chunk goes past the announced length.
        TooLarge,
        /// CRC-32 of the received bytes differs from the announced one.
        CrcMismatch,
        /// Config can't be decoded or fails validation.
        Invalid,
        /// Flash read / write failed.
        Storage,
        /// Keyboard can't handle the request now, try again later.
        Busy,
    }

    impl fmt::Display for ErrorCode {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                ErrorCode::BadRequest => "bad request",
                ErrorCode::OutOfOrder => "request out of order",
                ErrorCode::TooLarge => "config too large",
                ErrorCode::CrcMismatch => "CRC mismatch",
                ErrorCode::Invalid => "invalid config",
                ErrorCode::Storage => "storage error",
                ErrorCode::Busy => "busy",
            })
        }
    }

    #[cfg(feature = "std")]
    impl std::error::Error for ErrorCode {}

    macro_rules! postcard_helpers {
        ($($ty:ty),*) => {$(
            impl $ty {
                /// Encodes with postcard.
                pub fn to_bytes(&self) -> Vec<u8> {
                    postcard::to_allocvec(self).expect("serialization to a vec can't fail")
                }

                /// Decodes from postcard.
                pub fn from_bytes(bytes: &[u8]) -> Result<Self, postcard::Error> {
                    postcard::from_bytes(bytes)
                }
            }
        )*};
    }

    postcard_helpers!(Info, Request, Response);

    impl From<Result<(), ErrorCode>> for Response {
        fn from(result: Result<(), ErrorCode>) -> Self {
            match result {
                Ok(()) => Response::Ok,
                Err(code) => Response::Error(code),
            }
        }
    }

    impl Response {
        /// Answer to [`Request::ReadConfig`]: at most `max_data` bytes of `config` (the encoded config) at `offset`.
        pub fn config_chunk(config: &[u8], offset: u32, max_data: usize) -> Response {
            let start = (offset as usize).min(config.len());
            let end = start.saturating_add(max_data).min(config.len());
            Response::ConfigChunk {
                total_len: config.len() as u32,
                offset,
                data: config[start..end].to_vec(),
            }
        }
    }

    /// Keyboard side of the write flow: assembles chunks and checks the result.
    ///
    /// Any error aborts the current transfer.
    #[derive(Debug)]
    pub struct ConfigReceiver {
        width: u8,
        height: u8,
        transfer: Option<Transfer>,
    }

    #[derive(Debug)]
    struct Transfer {
        len: usize,
        crc32: u32,
        bytes: Vec<u8>,
    }

    impl ConfigReceiver {
        /// Receiver for a keyboard with `width` x `height` keys, configs for other sizes are [`ErrorCode::Invalid`].
        pub fn new(width: u8, height: u8) -> Self {
            Self { width, height, transfer: None }
        }

        pub fn is_active(&self) -> bool {
            self.transfer.is_some()
        }

        /// Drops the current transfer (e.g. on disconnect).
        pub fn abort(&mut self) {
            self.transfer = None;
        }

        /// [`Request::WriteBegin`]: starts a new transfer, dropping the current one.
        pub fn begin(&mut self, len: u32, crc32: u32) -> Result<(), ErrorCode> {
            self.transfer = None;
            let len = len as usize;
            if len > MAX_CONFIG_SIZE {
                return Err(ErrorCode::TooLarge);
            }
            if len == 0 {
                return Err(ErrorCode::BadRequest);
            }
            self.transfer = Some(Transfer { len, crc32, bytes: Vec::with_capacity(len) });
            Ok(())
        }

        /// [`Request::WriteChunk`]: appends `data`, `offset` must equal the number of bytes received so far.
        pub fn chunk(&mut self, offset: u32, data: &[u8]) -> Result<(), ErrorCode> {
            let result = self.try_chunk(offset, data);
            if result.is_err() {
                self.transfer = None;
            }
            result
        }

        fn try_chunk(&mut self, offset: u32, data: &[u8]) -> Result<(), ErrorCode> {
            let transfer = self.transfer.as_mut().ok_or(ErrorCode::OutOfOrder)?;
            if offset as usize != transfer.bytes.len() {
                return Err(ErrorCode::OutOfOrder);
            }
            if transfer.bytes.len() + data.len() > transfer.len {
                return Err(ErrorCode::TooLarge);
            }
            transfer.bytes.extend_from_slice(data);
            Ok(())
        }

        /// [`Request::WriteCommit`]: checks length and CRC, decodes and validates the config.
        /// Ends the transfer whatever the result is.
        pub fn commit(&mut self) -> Result<Config, ErrorCode> {
            let transfer = self.transfer.take().ok_or(ErrorCode::OutOfOrder)?;
            if transfer.bytes.len() != transfer.len {
                return Err(ErrorCode::OutOfOrder);
            }
            if crc32(&transfer.bytes) != transfer.crc32 {
                return Err(ErrorCode::CrcMismatch);
            }
            let config = Config::from_bytes(&transfer.bytes).map_err(|_| ErrorCode::Invalid)?;
            config.validate_for(self.width, self.height).map_err(|_| ErrorCode::Invalid)?;
            Ok(config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_strings_match() {
        for (uuid, text) in [
            (SERVICE_UUID, SERVICE_UUID_STR),
            (INFO_UUID, INFO_UUID_STR),
            (CONTROL_UUID, CONTROL_UUID_STR),
            (RESPONSE_UUID, RESPONSE_UUID_STR),
        ] {
            let hex: std::string::String = text.chars().filter(|c| *c != '-').collect();
            assert_eq!(u128::from_str_radix(&hex, 16).unwrap(), uuid);
        }
    }

    #[test]
    fn chunks_cover_bytes() {
        let bytes: std::vec::Vec<u8> = (0..=255).collect();
        let parts: std::vec::Vec<_> = chunks(&bytes, 100).collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2].0, 200);
        assert_eq!(parts.iter().map(|(_, c)| c.len()).sum::<usize>(), 256);
        assert_eq!(chunk_payload_size(DEFAULT_MTU), 7);
        assert_eq!(chunk_payload_size(2), 0);
    }

    #[cfg(feature = "alloc")]
    mod messages {
        use alloc::vec;
        use alloc::vec::Vec;

        use super::super::*;
        use crate::config::tests::sample;
        use crate::crc::crc32;

        fn send(receiver: &mut ConfigReceiver, bytes: &[u8], mtu: usize) -> Result<crate::config::Config, ErrorCode> {
            receiver.begin(bytes.len() as u32, crc32(bytes))?;
            for (offset, chunk) in chunks(bytes, chunk_payload_size(mtu)) {
                receiver.chunk(offset, chunk)?;
            }
            receiver.commit()
        }

        #[test]
        fn messages_fit_mtu() {
            for mtu in [DEFAULT_MTU, 185, 247, 517] {
                let size = chunk_payload_size(mtu);
                let request = Request::WriteChunk { offset: u32::MAX, data: vec![0xFF; size] };
                assert!(request.to_bytes().len() <= mtu - ATT_HEADER, "mtu {mtu}");
                let response = Response::ConfigChunk { total_len: u32::MAX, offset: u32::MAX, data: vec![0xFF; size] };
                assert!(response.to_bytes().len() <= mtu - ATT_HEADER, "mtu {mtu}");
            }
        }

        #[test]
        fn messages_roundtrip() {
            let info = Info {
                protocol_version: PROTOCOL_VERSION,
                config_version: crate::config::CONFIG_VERSION,
                width: 10,
                height: 5,
                max_layouts: crate::config::MAX_LAYOUTS as u8,
                max_combos: crate::config::MAX_COMBOS as u8,
                firmware_version: "0.1.0".into(),
            };
            assert_eq!(Info::from_bytes(&info.to_bytes()).unwrap(), info);

            for request in [
                Request::GetInfo,
                Request::ReadConfig { offset: 300 },
                Request::WriteBegin { len: 500, crc32: 0xDEAD_BEEF },
                Request::WriteChunk { offset: 7, data: vec![1, 2, 3] },
                Request::WriteCommit,
                Request::ResetToDefault,
                Request::Ping,
                Request::End,
            ] {
                assert_eq!(Request::from_bytes(&request.to_bytes()).unwrap(), request);
            }

            for response in [
                Response::Info(info),
                Response::ConfigChunk { total_len: 10, offset: 4, data: vec![9; 6] },
                Response::Ok,
                Response::Error(ErrorCode::CrcMismatch),
            ] {
                assert_eq!(Response::from_bytes(&response.to_bytes()).unwrap(), response);
            }
            assert!(Request::from_bytes(&[0xFF]).is_err());
        }

        #[test]
        fn read_config_in_chunks() {
            let bytes = sample().to_bytes();
            let max_data = chunk_payload_size(DEFAULT_MTU);
            let mut received = Vec::new();
            loop {
                let Response::ConfigChunk { total_len, offset, data } =
                    Response::config_chunk(&bytes, received.len() as u32, max_data) else { unreachable!() };
                assert_eq!(total_len as usize, bytes.len());
                assert_eq!(offset as usize, received.len());
                assert!(data.len() <= max_data);
                if data.is_empty() {
                    break;
                }
                received.extend_from_slice(&data);
            }
            assert_eq!(received, bytes);
            assert_eq!(
                Response::config_chunk(&bytes, u32::MAX, max_data),
                Response::ConfigChunk { total_len: bytes.len() as u32, offset: u32::MAX, data: vec![] }
            );
        }

        #[test]
        fn receiver_happy_path() {
            let config = sample();
            let bytes = config.to_bytes();
            let mut receiver = ConfigReceiver::new(10, 5);
            for mtu in [DEFAULT_MTU, 247] {
                assert_eq!(send(&mut receiver, &bytes, mtu), Ok(config.clone()));
                assert!(!receiver.is_active());
            }
        }

        #[test]
        fn receiver_out_of_order() {
            let bytes = sample().to_bytes();
            let mut receiver = ConfigReceiver::new(10, 5);
            assert_eq!(receiver.chunk(0, &bytes), Err(ErrorCode::OutOfOrder));
            assert_eq!(receiver.commit(), Err(ErrorCode::OutOfOrder));

            receiver.begin(bytes.len() as u32, crc32(&bytes)).unwrap();
            receiver.chunk(0, &bytes[..10]).unwrap();
            assert_eq!(receiver.chunk(20, &bytes[20..30]), Err(ErrorCode::OutOfOrder));
            assert!(!receiver.is_active());

            receiver.begin(bytes.len() as u32, crc32(&bytes)).unwrap();
            receiver.chunk(0, &bytes[..10]).unwrap();
            assert_eq!(receiver.commit(), Err(ErrorCode::OutOfOrder));

            // A new begin restarts the transfer.
            receiver.begin(bytes.len() as u32, crc32(&bytes)).unwrap();
            receiver.chunk(0, &bytes[..10]).unwrap();
            assert_eq!(send(&mut receiver, &bytes, 247), Ok(sample()));
        }

        #[test]
        fn receiver_crc_mismatch() {
            let bytes = sample().to_bytes();
            let mut receiver = ConfigReceiver::new(10, 5);
            receiver.begin(bytes.len() as u32, crc32(&bytes) ^ 1).unwrap();
            receiver.chunk(0, &bytes).unwrap();
            assert_eq!(receiver.commit(), Err(ErrorCode::CrcMismatch));
            assert!(!receiver.is_active());
        }

        #[test]
        fn receiver_too_large() {
            let bytes = sample().to_bytes();
            let mut receiver = ConfigReceiver::new(10, 5);
            assert_eq!(receiver.begin(MAX_CONFIG_SIZE as u32 + 1, 0), Err(ErrorCode::TooLarge));
            assert_eq!(receiver.begin(0, 0), Err(ErrorCode::BadRequest));

            receiver.begin(10, crc32(&bytes)).unwrap();
            assert_eq!(receiver.chunk(0, &bytes[..11]), Err(ErrorCode::TooLarge));
            assert!(!receiver.is_active());
        }

        #[test]
        fn receiver_invalid() {
            let mut receiver = ConfigReceiver::new(10, 5);

            let garbage = [0xFFu8; 16];
            assert_eq!(send(&mut receiver, &garbage, 247), Err(ErrorCode::Invalid));

            let mut config = sample();
            config.combos[0].action = crate::config::ComboAction::SetLayout(7);
            assert_eq!(send(&mut receiver, &config.to_bytes(), 247), Err(ErrorCode::Invalid));

            let mut receiver = ConfigReceiver::new(12, 5);
            assert_eq!(send(&mut receiver, &sample().to_bytes(), 247), Err(ErrorCode::Invalid));
        }
    }
}
