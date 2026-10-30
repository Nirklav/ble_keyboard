//! Firmware update of a connected keyboard, over the update part of the protocol (see [`protocol::hid`]):
//! works for a firmware of any protocol version.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use protocol::crc::crc32;
use protocol::hid::{self, ErrorCode, Request, Response, Versions};
use protocol::image;

use crate::compat::check_update;
use crate::link::{Client, NotSent, Transport};
use crate::time::sleep;

/// Progress of [`update_firmware`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareProgress {
    /// `done` of `total` compressed bytes are sent.
    Sent { done: usize, total: usize },
    /// All bytes are sent, the keyboard checks the image; then it restarts.
    Checking,
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
    device: &mut Client<impl Transport>,
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
pub async fn reconnect_after_update<T: Transport>(
    device_id: [u8; 6],
    mut connect: impl AsyncFnMut() -> Result<Client<T>>,
) -> Result<(Client<T>, Versions)> {
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
                protocol::firmware::ROLLBACK_TIMEOUT
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

    impl Transport for WindowDevice {
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
        let mut client = Client::new(WindowDevice::new(None));
        let mut done = Vec::new();
        let sent = block_on(update_firmware(&mut client, &versions, &firmware(&image), |progress| {
            if let FirmwareProgress::Sent { done: bytes, .. } = progress {
                done.push(bytes);
            }
        }))
        .unwrap();
        let device = client.transport();
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
        let mut client = Client::new(WindowDevice::new(Some(5)));
        let mut answered = 0;
        let error = block_on(update_firmware(&mut client, &versions, &firmware(&image), |progress| {
            if let FirmwareProgress::Sent { .. } = progress {
                answered += 1;
            }
        }))
        .unwrap_err();
        assert_eq!(error.downcast_ref::<ErrorCode>(), Some(&ErrorCode::Storage));
        let device = client.transport();
        assert_eq!(answered, 5);
        // The window was full when the error came, nothing was sent after it.
        assert_eq!(device.chunks, 5 + hid::CHUNK_WINDOW);
        // The responses of the chunks on the way are taken, not left for the next request.
        assert!(device.responses.is_empty());
    }
}
