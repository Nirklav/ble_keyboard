//! Config protocol over the HID config reports, see [`protocol::hid`] for the reports and the message flows.
//!
//! Registered on the [`ConfigReports`] of the HID device. GATT callbacks run in the NimBLE host task and never
//! touch [`Services`](crate::services::Services):
//! they only use [`State`] behind a mutex. Requests that change the config ([`Request::WriteCommit`],
//! [`Request::ResetToDefault`]) are handed to the main loop through [`ConfigService::take_submitted`],
//! the main loop applies them once all keys are up and answers with [`ConfigService::finish`].
//! Firmware requests and [`Request::GetVersions`] go to the [`FirmwareUpdate`] thread, it answers them.

use std::sync::{Arc, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use log::{info, warn};
use protocol::hid::{self, ConfigReceiver, ErrorCode, Info, Request, Response};
use protocol::config::{CONFIG_VERSION, MAX_COMBOS, MAX_LAYOUTS};

use crate::bt::ble_mouse_keyboard::{ConfigReplies, ConfigReports, ConnectionId};
use crate::config::Pending;
use crate::firmware::FirmwareUpdate;
use crate::sleep_admission::{SleepReservation, SLEEP};

/// The keyboard stays active (no idle connection parameters) this long after the last transfer request.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a response of the firmware thread waits for NimBLE buffers: the app sends up to
/// [`hid::CHUNK_WINDOW`] chunks at once, their answers may come faster than the link sends them.
const SEND_RETRY_TIMEOUT: Duration = Duration::from_secs(2);
const SEND_RETRY_DELAY: Duration = Duration::from_millis(5);

/// Shared between the GATT callbacks and the main loop.
#[derive(Clone)]
pub struct ConfigService {
    state: Arc<std::sync::Mutex<State>>,
    replies: ConfigReplies,
    on_activity: Arc<dyn Fn() + Send + Sync>,
    info: Arc<Info>,
    firmware: FirmwareUpdate,
}

/// Owns the origin and sleep reservation from admission through the response attempt.
pub(crate) struct RequestContext {
    origin: ConnectionId,
    sleep: SleepReservation<'static>,
}

pub(crate) struct Submitted {
    pub(crate) change: Pending,
    pub(crate) context: RequestContext,
}

struct State {
    /// Connection that owns the receiver and session lease.
    origin: Option<ConnectionId>,
    receiver: ConfigReceiver,
    /// Encoded active config, answers [`Request::ReadConfig`].
    current: Vec<u8>,
    /// Config change for the main loop.
    submitted: Option<Submitted>,
    /// A change is submitted or being applied, its response is not sent yet.
    busy: bool,
    /// Last request except [`Request::Ping`] and [`Request::End`].
    last_request: Option<Instant>,
    /// The client started a session and has not ended it.
    session_open: bool,
}

impl State {
    fn clear_client(&mut self) {
        if self.receiver.is_active() {
            info!("Config transfer aborted");
        }
        self.receiver.abort();
        self.origin = None;
        self.last_request = None;
        self.session_open = false;
    }
}

enum Outcome {
    Reply(Response),
    Apply(Pending),
    Firmware(Request),
}

impl ConfigService {
    /// Must be called before the server starts ([`BleMouseKeyboard::start`](crate::bt::ble_mouse_keyboard::BleMouseKeyboard::start)).
    /// `current` is the encoded active config. `on_activity` runs in the NimBLE task or the firmware
    /// update thread and must return promptly.
    pub fn register(
        width: u8,
        height: u8,
        current: Vec<u8>,
        on_activity: impl Fn() + Send + Sync + 'static,
        reports: ConfigReports,
    ) -> anyhow::Result<Self> {
        let ConfigReports { output, replies } = reports;
        let info = Info {
            protocol_version: hid::PROTOCOL_VERSION,
            config_version: CONFIG_VERSION,
            width,
            height,
            max_layouts: MAX_LAYOUTS as u8,
            max_combos: MAX_COMBOS as u8,
            firmware_version: env!("CARGO_PKG_VERSION").into(),
        };

        let on_activity: Arc<dyn Fn() + Send + Sync> = Arc::new(on_activity);
        let firmware_replies = replies.clone();
        let on_update = on_activity.clone();
        let firmware = FirmwareUpdate::spawn(
            move |origin, response| send(&firmware_replies, origin, response, SEND_RETRY_TIMEOUT),
            move || on_update(),
        )?;

        let config_service = Self {
            state: Arc::new(std::sync::Mutex::new(State {
                origin: None,
                receiver: ConfigReceiver::new(width, height),
                current,
                submitted: None,
                busy: false,
                last_request: None,
                session_open: false,
            })),
            replies,
            on_activity,
            info: Arc::new(info),
            firmware,
        };

        let callback_service = config_service.clone();
        output.lock().on_write(move |args| {
            let Some(sleep) = SLEEP.reserve() else {
                args.reject_with_error_code(esp_idf_sys::BLE_ATT_ERR_WRITE_NOT_PERMITTED as u8);
                return;
            };
            let Some(origin) = callback_service.replies.capture(args.desc()) else {
                args.reject_with_error_code(esp_idf_sys::BLE_ATT_ERR_WRITE_NOT_PERMITTED as u8);
                return;
            };
            let context = RequestContext { origin, sleep };
            let mtu = args.desc().mtu() as usize;
            if mtu < hid::MIN_MTU {
                warn!("Config report over MTU {}, responses don't fit (needs {})", mtu, hid::MIN_MTU);
            }
            match hid::decode(args.recv_data()) {
                Some(data) => callback_service.on_request(data, context),
                None => {
                    warn!("Config report: bad length {:02x?}", args.recv_data());
                    callback_service.send(origin, &Response::Error(ErrorCode::BadRequest));
                },
            }
        });

        Ok(config_service)
    }

    /// Handles a request in the NimBLE task (the output report is locked, the input one is not).
    /// Session requests and firmware chunks skip the activity callback so the keyboard can idle and OTA chunks
    /// don't wake the main loop hundreds of times per second. Firmware update transitions report activity separately.
    fn on_request(&self, data: &[u8], context: RequestContext) {
        let Ok(request) = Request::from_bytes(data) else {
            warn!("Config service: bad request {:02x?}", data);
            self.send(context.origin, &Response::Error(ErrorCode::BadRequest));
            (self.on_activity)();
            return;
        };
        self.firmware.link_works(&context.sleep);

        let mut state = self.lock();
        if state.origin != Some(context.origin) {
            state.clear_client();
            state.origin = Some(context.origin);
        }
        let now = Instant::now();
        let (outcome, notify_activity) = if request == Request::End {
            info!("Config session ended");
            // An unfinished firmware transfer ends with the session.
            self.firmware.abort(context.origin, &context.sleep);
            state.session_open = false;
            state.last_request = None;
            (Outcome::Reply(Response::Ok), false)
        } else {
            if !state.session_open {
                info!("Config session started");
                state.session_open = true;
            }
            if request == Request::Ping {
                (Outcome::Reply(Response::Ok), false)
            } else {
                state.last_request = Some(now);
                let notify_activity = !matches!(request, Request::FirmwareChunk { .. });
                (self.handle_transfer(&mut state, request), notify_activity)
            }
        };
        context.sleep.set_session_expiry(state.session_open.then(|| now + hid::SESSION_TIMEOUT));
        match outcome {
            Outcome::Reply(response) => {
                drop(state);
                self.send(context.origin, &response);
                if notify_activity {
                    (self.on_activity)();
                }
            },
            Outcome::Apply(change) => {
                let _publication = context.sleep.retain();
                state.busy = true;
                state.submitted = Some(Submitted { change, context });
                drop(state);
                (self.on_activity)();
            },
            Outcome::Firmware(request) => {
                // The worker may finish before the callback publishes activity.
                let _publication = notify_activity.then(|| context.sleep.retain());
                drop(state);
                self.firmware.request(context.origin, request, context.sleep);
                if notify_activity {
                    (self.on_activity)();
                }
            },
        }
    }

    fn handle_transfer(&self, state: &mut State, request: Request) -> Outcome {
        if state.busy && !matches!(request, Request::GetInfo | Request::GetVersions) {
            return Outcome::Reply(Response::Error(ErrorCode::Busy));
        }

        let response = match request {
            Request::GetInfo => Response::Info((*self.info).clone()),
            Request::ReadConfig { offset } => {
                Response::config_chunk(&state.current, offset, hid::CHUNK_SIZE)
            },
            Request::WriteBegin { len, crc32 } => {
                info!("Config write started: {} bytes", len);
                state.receiver.begin(len, crc32).into()
            },
            Request::WriteChunk { offset, data } => state.receiver.chunk(offset, &data).into(),
            Request::WriteCommit => match state.receiver.commit() {
                Ok(config) => {
                    info!("Config received");
                    return Outcome::Apply(Pending::Apply(config));
                },
                Err(code) => Response::Error(code),
            },
            Request::ResetToDefault => {
                info!("Config reset requested");
                state.receiver.abort();
                return Outcome::Apply(Pending::Reset);
            },
            // The versions too: the build ID takes a read of the whole image.
            Request::GetVersions
            | Request::FirmwareBegin { .. }
            | Request::FirmwareChunk { .. }
            | Request::FirmwareCommit => return Outcome::Firmware(request),
            Request::Ping | Request::End => unreachable!("handled by the caller"),
        };

        if let Response::Error(code) = &response {
            warn!("Config request failed: {}", code);
        }
        Outcome::Reply(response)
    }

    /// Config change submitted by the app, to be applied by the main loop and answered with [`Self::finish`].
    pub(crate) fn take_submitted(&self) -> Option<Submitted> {
        self.lock().submitted.take()
    }

    /// A config request came recently, the host is likely in the middle of a transfer.
    pub fn transfer_active(&self) -> bool {
        self.lock()
            .last_request
            .is_some_and(|t| t.elapsed() < TRANSFER_TIMEOUT)
    }

    /// Replaces the encoded active config, after any config change.
    pub fn set_current(&self, current: Vec<u8>) {
        self.lock().current = current;
    }

    /// Answers the change taken with [`Self::take_submitted`].
    pub(crate) fn finish(&self, context: RequestContext, response: Response) {
        self.lock().busy = false;
        self.send(context.origin, &response);
    }

    /// Drops an unfinished transfer, a submitted change is still applied.
    pub(crate) fn on_disconnect(&self, origin: ConnectionId, sleep: &SleepReservation<'static>) {
        let mut state = self.lock();
        if state.origin == Some(origin) {
            state.clear_client();
            sleep.set_session_expiry(None);
        }
        drop(state);
        self.firmware.abort(origin, sleep);
    }

    /// No retries: the buffers are freed by the NimBLE task, the caller may be it.
    fn send(&self, origin: ConnectionId, response: &Response) {
        send(&self.replies, origin, response, Duration::ZERO);
    }

    /// The state stays consistent if a holder panicked: every update is a single assignment.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Notifies `response` in the config input report to its origin, retrying up to `retry` while NimBLE
/// is out of buffers.
fn send(replies: &ConfigReplies, origin: ConnectionId, response: &Response, retry: Duration) {
    let bytes = response.to_bytes();
    // Fits a report: chunks are sized by it, other responses are small.
    let Some(report) = hid::encode(&bytes) else {
        warn!("Config response of {} bytes doesn't fit a report", bytes.len());
        return;
    };
    let start = Instant::now();
    loop {
        match replies.send(origin, &report) {
            None | Some(Ok(())) => return,
            Some(Err(e)) if e.code() == esp_idf_sys::BLE_HS_ENOMEM && start.elapsed() < retry => {
                thread::sleep(SEND_RETRY_DELAY);
            },
            Some(Err(e)) => {
                warn!("Config response dropped: {}", e);
                return;
            },
        }
    }
}
