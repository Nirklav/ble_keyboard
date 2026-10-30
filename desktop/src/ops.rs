//! Config and firmware operations on a connected keyboard, shared by the CLI and the GUI.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use protocol::hid::{self, ErrorCode, Info, Request, Response, Versions};
use protocol::config::{self, Config};
use protocol::crc::crc32;
use protocol::{firmware, image};

use crate::config_file;
use crate::link::{Link, NotSent};
use crate::time::sleep;

/// Progress of [`write_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteProgress {
    /// `done` of `total` bytes are sent.
    Sent { done: usize, total: usize },
    /// All bytes are sent, the keyboard is applying the config (waits until all keys are released).
    Applying,
}

/// Progress of [`update_firmware`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareProgress {
    /// `done` of `total` compressed bytes are sent.
    Sent { done: usize, total: usize },
    /// All bytes are sent, the keyboard checks the image; then it restarts.
    Checking,
}

/// Keyboard size used for validation while no keyboard is connected.
pub const DEFAULT_SIZE: (u8, u8) = (10, 5);

/// Checks what blocks a write: validity for the keyboard size, the keyboard limits, the encoded size.
/// `info` is the connected keyboard, if any.
pub fn check(config: &Config, info: Option<&Info>) -> Result<(), String> {
    let (width, height) = info.map_or(DEFAULT_SIZE, |info| (info.width, info.height));
    config.validate_for(width, height).map_err(|e| config_file::describe(&e, config))?;
    if let Some(info) = info {
        if config.layouts.len() > info.max_layouts as usize {
            return Err(format!("the keyboard supports at most {} layouts", info.max_layouts));
        }
        if config.combos.len() > info.max_combos as usize {
            return Err(format!("the keyboard supports at most {} combos", info.max_combos));
        }
    }
    let size = config.to_bytes().len();
    if size > hid::MAX_CONFIG_SIZE {
        return Err(format!("encoded config is {size} bytes, max is {}", hid::MAX_CONFIG_SIZE));
    }
    Ok(())
}

/// Fails if the firmware speaks a different protocol version.
pub fn check_protocol(protocol_version: u16) -> Result<()> {
    ensure!(
        protocol_version == hid::PROTOCOL_VERSION,
        "keyboard protocol version {protocol_version} differs from the app one {}, update the firmware or the app",
        hid::PROTOCOL_VERSION
    );
    Ok(())
}

/// Fails if the app can't install `firmware` on the keyboard: its update part of the protocol is another one,
/// the image is for another board or doesn't fit the update slot.
pub fn check_update(versions: &Versions, firmware: &Firmware) -> Result<()> {
    ensure!(
        versions.update_version != 0,
        "keyboard firmware {} has no update support yet, flash it over USB once",
        versions.firmware_version
    );
    ensure!(
        versions.update_version == hid::UPDATE_VERSION,
        "keyboard firmware {} updates with protocol version {}, the app with {}: flash it over USB",
        versions.firmware_version,
        versions.update_version,
        hid::UPDATE_VERSION
    );
    ensure!(
        firmware.board == versions.board,
        "the firmware is for board `{}`, the keyboard is `{}`",
        firmware.board,
        versions.board
    );
    ensure!(
        firmware.image.len() <= versions.slot_size as usize,
        "the firmware is {} bytes, the keyboard has room for {}",
        firmware.image.len(),
        versions.slot_size
    );
    Ok(())
}

/// Info of a just connected keyboard: the versions first, the info only if the firmware speaks the app protocol.
pub async fn connect_info(device: &mut impl Link) -> Result<Info> {
    let versions = device.versions().await?;
    check_protocol(versions.protocol_version)?;
    device.info().await
}

/// Fails if the app can't read the keyboard config version (a newer firmware, or one older than the upgrades go).
pub fn check_config_version(info: &Info) -> Result<()> {
    ensure!(
        (config::OLDEST_CONFIG_VERSION..=config::CONFIG_VERSION).contains(&info.config_version),
        "keyboard config version {} is not supported by the app (versions {}..={}), update the firmware or the app",
        info.config_version,
        config::OLDEST_CONFIG_VERSION,
        config::CONFIG_VERSION
    );
    Ok(())
}

/// Reads, decodes and validates the keyboard config, an older version is upgraded to the app one.
pub async fn read_config(device: &mut impl Link, info: &Info) -> Result<Config> {
    check_protocol(info.protocol_version)?;
    check_config_version(info)?;
    let bytes = read_config_bytes(device).await?;
    let config = Config::from_bytes(&bytes).context("can't decode the keyboard config")?;
    config
        .validate_for(info.width, info.height)
        .map_err(|e| anyhow!("keyboard config is invalid: {}", config_file::describe(&e, &config)))?;
    Ok(config)
}

async fn read_config_bytes(device: &mut impl Link) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut expected_len = None;
    loop {
        let offset = bytes.len() as u32;
        let response = device.request(&Request::ReadConfig { offset }).await?;
        let Response::ConfigChunk { total_len, offset: chunk_offset, data } = response else {
            bail!("unexpected response to ReadConfig: {response:?}");
        };
        ensure!(chunk_offset == offset, "asked for offset {offset}, got {chunk_offset}");
        ensure!(total_len as usize <= hid::MAX_CONFIG_SIZE, "config of {total_len} bytes is too large");
        if *expected_len.get_or_insert(total_len) != total_len {
            bail!("config changed while reading, try again");
        }

        bytes.extend_from_slice(&data);
        ensure!(bytes.len() <= total_len as usize, "keyboard sent more than {total_len} bytes");
        if bytes.len() == total_len as usize {
            return Ok(bytes);
        }
        ensure!(!data.is_empty(), "keyboard sent an empty chunk at {offset} of {total_len}");
    }
}

/// Validates `config` against the keyboard first. Returns the number of bytes sent.
pub async fn write_config(
    device: &mut impl Link,
    info: &Info,
    config: &Config,
    mut progress: impl FnMut(WriteProgress),
) -> Result<usize> {
    check_protocol(info.protocol_version)?;
    ensure!(
        info.config_version == config::CONFIG_VERSION,
        "keyboard config version {} differs from the app one {}, update the firmware or the app",
        info.config_version,
        config::CONFIG_VERSION
    );
    check(config, Some(info)).map_err(anyhow::Error::msg)?;

    let bytes = config.to_bytes();
    let total = bytes.len();
    device.request_ok(&Request::WriteBegin { len: total as u32, crc32: crc32(&bytes) }).await?;
    for (offset, chunk) in hid::chunks(&bytes, hid::CHUNK_SIZE) {
        device.request_ok(&Request::WriteChunk { offset, data: chunk.to_vec() }).await?;
        progress(WriteProgress::Sent { done: offset as usize + chunk.len(), total });
    }
    progress(WriteProgress::Applying);
    device.request_ok(&Request::WriteCommit).await?;
    Ok(total)
}

/// Restores the factory config on the keyboard.
pub async fn reset_config(device: &mut impl Link, info: &Info) -> Result<()> {
    check_protocol(info.protocol_version)?;
    device.request_ok(&Request::ResetToDefault).await
}

/// Wait before the first reconnect: the keyboard answers the commit and restarts half a second later,
/// and the host needs a moment to notice the new connection.
const RESTART_WAIT: Duration = Duration::from_secs(3);
/// How long [`reconnect_after_update`] tries.
const RECONNECT_TRIES: u32 = 45;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

/// A keyboard firmware image (`espflash save-image` output) to install.
#[derive(Debug, Clone)]
pub struct Firmware<'a> {
    pub image: &'a [u8],
    pub version: String,
    /// Board it's built for, see [`protocol::image`].
    pub board: String,
    /// [`image::build_id`].
    pub build_id: [u8; image::BUILD_ID_LEN],
}

impl Firmware<'_> {
    /// Checks that `image` is a keyboard firmware, of some board: [`check_update`] tells whether it's the keyboard's.
    pub fn parse(image: &[u8]) -> Result<Firmware<'_>> {
        let app =
            image::app_info(image).context("not a firmware image (an ELF file? make one with `espflash save-image`)")?;
        ensure!(
            app.chip_id == image::CHIP_ESP32C3,
            "not a keyboard firmware: project `{}`, chip {:#06x}",
            app.project_name,
            app.chip_id
        );
        let build_id = image::build_id(image).context("the firmware image has no hash at its end")?;
        Ok(Firmware { image, version: app.version.to_string(), board: app.project_name.to_string(), build_id })
    }

    /// Whether the keyboard runs this very build, not only its version. Only the version tells when the
    /// keyboard doesn't know its build ID (zeros: a firmware that can't read its hash, or one without the ID).
    pub fn runs_on(&self, versions: &Versions) -> bool {
        versions.firmware_version == self.version
            && (versions.build_id == [0; image::BUILD_ID_LEN] || versions.build_id == self.build_id)
    }

    /// After an update: fails unless the keyboard runs this build.
    pub fn check_running(&self, versions: &Versions) -> Result<()> {
        ensure!(
            self.runs_on(versions),
            "the keyboard runs firmware {} (build {}), not {} (build {}): the update didn't take, or the new firmware \
             didn't start and the keyboard went back to the old one",
            versions.firmware_version,
            hex(&versions.build_id),
            self.version,
            hex(&self.build_id)
        );
        Ok(())
    }
}

/// The firmware bundled with the app, if it has one ([`crate::BUNDLED_FIRMWARE`]).
pub fn bundled_firmware() -> Option<&'static Firmware<'static>> {
    static BUNDLED: OnceLock<Option<Firmware<'static>>> = OnceLock::new();
    // build.rs checked it.
    let parse = || crate::BUNDLED_FIRMWARE.map(|image| Firmware::parse(image).expect("the bundled firmware is valid"));
    BUNDLED.get_or_init(parse).as_ref()
}

/// [`Versions::device_id`] for messages: the MAC address.
pub fn device_id_text(id: &[u8; 6]) -> String {
    id.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(":")
}

/// Build ID for messages.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Sends `firmware` to the keyboard, which restarts into it once it's checked; [`Firmware::check_running`]
/// after [`reconnect_after_update`] tells it started. Returns the number of compressed bytes sent.
/// Uses only the update part of the protocol: works for a firmware of any protocol version.
pub async fn update_firmware(
    device: &mut impl Link,
    versions: &Versions,
    firmware: &Firmware<'_>,
    mut progress: impl FnMut(FirmwareProgress),
) -> Result<usize> {
    check_update(versions, firmware)?;

    let image = firmware.image;
    let bytes = protocol::firmware::compress(image);
    let total = bytes.len();
    let begin = Request::FirmwareBegin { len: total as u32, crc32: crc32(&bytes), image_len: image.len() as u32 };
    device.request_ok(&begin).await?;
    // Up to `CHUNK_WINDOW` chunks on the way, the ends of the unanswered ones.
    let mut chunks = hid::chunks(&bytes, hid::CHUNK_SIZE);
    let mut unanswered = VecDeque::with_capacity(hid::CHUNK_WINDOW);
    loop {
        while unanswered.len() < hid::CHUNK_WINDOW {
            let Some((offset, chunk)) = chunks.next() else { break };
            device.send(&Request::FirmwareChunk { offset, data: chunk.to_vec() }).await?;
            unanswered.push_back(offset as usize + chunk.len());
        }
        let Some(done) = unanswered.pop_front() else { break };
        match device.receive("FirmwareChunk").await? {
            Response::Ok => progress(FirmwareProgress::Sent { done, total }),
            Response::Error(code) => {
                // The chunks on the way fail too: their responses would answer the next requests.
                for _ in unanswered {
                    if device.receive("FirmwareChunk").await.is_err() {
                        break;
                    }
                }
                return Err(code).context("FirmwareChunk failed");
            }
            response => bail!("unexpected response to FirmwareChunk: {response:?}"),
        }
    }
    progress(FirmwareProgress::Checking);
    match device.request_ok(&Request::FirmwareCommit).await {
        // The answer of a commit that went through may be lost in the restart: the reconnect tells which
        // firmware runs. Only a refusal of the keyboard, or a commit that never left, is sure to have changed nothing.
        Err(error) if error.downcast_ref::<ErrorCode>().is_some() || error.downcast_ref::<NotSent>().is_some() => {
            Err(error)
        }
        _ => Ok(total),
    }
}

/// Reconnects with `connect` after [`update_firmware`] of the keyboard with `device_id`
/// ([`Versions::device_id`]), the keyboard restarts into the new firmware; the first request confirms it.
/// Returns the connection and the versions, the firmware version tells whether the new firmware runs or
/// the keyboard went back to the old one.
pub async fn reconnect_after_update<D: Link>(
    device_id: [u8; 6],
    mut connect: impl AsyncFnMut() -> Result<D>,
) -> Result<(D, Versions)> {
    sleep(RESTART_WAIT).await;
    let mut tries = 0;
    loop {
        // A connection to the old device (gone with the restart) fails its request, the next try finds the new one.
        let error = match connect().await {
            Ok(mut device) => match device.versions().await {
                Ok(versions) if versions.device_id == device_id => return Ok((device, versions)),
                // `connect` takes the first keyboard it finds, it would find this one again.
                Ok(versions) => {
                    device.close().await;
                    bail!(
                        "another keyboard ({}) answered after the update, not the updated one ({}): disconnect the \
                         other one and connect to the updated one, which confirms its new firmware",
                        device_id_text(&versions.device_id),
                        device_id_text(&device_id)
                    );
                }
                Err(error) => error,
            },
            Err(error) => error,
        };
        tries += 1;
        if tries == RECONNECT_TRIES {
            // Not a duration: a try that reaches a stale device waits for the request timeout too.
            return Err(error.context(format!(
                "the keyboard didn't come back in {RECONNECT_TRIES} tries after the update; once it reconnects, connecting \
                 to it (`kb info`, the app) confirms the new firmware, without one it goes back to the old firmware {:?} after the \
                 restart",
                firmware::ROLLBACK_TIMEOUT
            )));
        }
        sleep(RECONNECT_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    /// Keyboard that takes the chunks in order and answers them only when asked; fails chunk `fail_at`.
    struct WindowDevice {
        received: usize,
        /// Responses of the sent requests, the next one first.
        responses: VecDeque<Response>,
        max_unanswered: usize,
        fail_at: Option<usize>,
        failed: bool,
        chunks: usize,
    }

    impl WindowDevice {
        fn new(fail_at: Option<usize>) -> Self {
            WindowDevice { received: 0, responses: VecDeque::new(), max_unanswered: 0, fail_at, failed: false, chunks: 0 }
        }
    }

    impl Link for WindowDevice {
        fn drain(&mut self) -> Result<()> {
            self.responses.clear();
            Ok(())
        }

        async fn send(&mut self, request: &Request) -> Result<()> {
            let response = match request {
                Request::FirmwareBegin { .. } | Request::FirmwareCommit => Response::Ok,
                Request::FirmwareChunk { offset, data } => {
                    let fail = self.fail_at == Some(self.chunks);
                    self.chunks += 1;
                    if self.failed || *offset as usize != self.received {
                        Response::Error(ErrorCode::OutOfOrder)
                    } else if fail {
                        self.failed = true;
                        Response::Error(ErrorCode::Storage)
                    } else {
                        self.received += data.len();
                        Response::Ok
                    }
                }
                request => bail!("unexpected request {request:?}"),
            };
            self.responses.push_back(response);
            self.max_unanswered = self.max_unanswered.max(self.responses.len());
            Ok(())
        }

        async fn receive(&mut self, name: &'static str) -> Result<Response> {
            self.responses.pop_front().with_context(|| format!("no response to {name}"))
        }

        async fn close(self) {}
    }

    fn versions() -> Versions {
        Versions {
            firmware_version: "0.0.0".into(),
            protocol_version: hid::PROTOCOL_VERSION,
            update_version: hid::UPDATE_VERSION,
            build_id: [0; image::BUILD_ID_LEN],
            board: image::KEYBOARD_PROJECT.into(),
            device_id: [0; 6],
            slot_size: 1 << 20,
            features: 0,
        }
    }

    /// Random-ish bytes, so that the compressed image takes many chunks.
    fn image() -> Vec<u8> {
        let mut state = 1u32;
        (0..20_000)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect()
    }

    fn firmware(image: &[u8]) -> Firmware<'_> {
        Firmware {
            image,
            version: "0.0.1".into(),
            board: image::KEYBOARD_PROJECT.into(),
            build_id: [0; image::BUILD_ID_LEN],
        }
    }

    #[test]
    fn update_sends_a_window_of_chunks() {
        let image = image();
        let versions = versions();
        let mut device = WindowDevice::new(None);
        let mut done = Vec::new();
        let sent = block_on(update_firmware(&mut device, &versions, &firmware(&image), |progress| {
            if let FirmwareProgress::Sent { done: bytes, .. } = progress {
                done.push(bytes);
            }
        }))
        .unwrap();
        assert_eq!(device.received, sent);
        assert_eq!(device.max_unanswered, hid::CHUNK_WINDOW);
        // Progress of the answered chunks, every one of them.
        assert_eq!(done.len(), sent.div_ceil(hid::CHUNK_SIZE));
        assert!(done.is_sorted());
        assert_eq!(done.last(), Some(&sent));
    }

    #[test]
    fn update_stops_at_the_first_failed_chunk() {
        let image = image();
        let versions = versions();
        let mut device = WindowDevice::new(Some(5));
        let mut answered = 0;
        let error = block_on(update_firmware(&mut device, &versions, &firmware(&image), |progress| {
            if let FirmwareProgress::Sent { .. } = progress {
                answered += 1;
            }
        }))
        .unwrap_err();
        assert_eq!(error.downcast_ref::<ErrorCode>(), Some(&ErrorCode::Storage));
        assert_eq!(answered, 5);
        // The window was full when the error came, nothing was sent after it.
        assert_eq!(device.chunks, 5 + hid::CHUNK_WINDOW);
        // The responses of the chunks on the way are taken, not left for the next request.
        assert!(device.responses.is_empty());
    }
}
