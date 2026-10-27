//! Config protocol over the HID config reports, see [`protocol::hid`] for the reports and the message flows.
//!
//! GATT callbacks run in the NimBLE host task and never touch [`Services`](crate::services::Services):
//! they only use [`State`] behind a mutex. Requests that change the config ([`Request::WriteCommit`],
//! [`Request::ResetToDefault`]) are handed to the main loop through [`ConfigService::take_submitted`],
//! the main loop applies them once all keys are up and answers with [`ConfigService::finish`].

use std::num::NonZeroU32;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

use esp32_nimble::utilities::mutex::Mutex;
use esp32_nimble::BLECharacteristic;
use esp_idf_svc::hal::task::notification::Notifier;
use log::{info, warn};
use protocol::hid::{self, ConfigReceiver, ErrorCode, Info, Request, Response};
use protocol::config::{CONFIG_VERSION, MAX_COMBOS, MAX_LAYOUTS};

use crate::config::Pending;

/// The keyboard stays active (no idle connection parameters) this long after the last transfer request.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared between the GATT callbacks and the main loop.
#[derive(Clone)]
pub struct ConfigService {
    state: Arc<std::sync::Mutex<State>>,
    /// HID config input report, responses to the requests of the output report.
    input: Arc<Mutex<BLECharacteristic>>,
    /// Wakes the main loop up from `wait_for_activity`.
    wake: Arc<Notifier>,
    info: Arc<Info>,
}

struct State {
    receiver: ConfigReceiver,
    /// Encoded active config, answers [`Request::ReadConfig`].
    current: Vec<u8>,
    /// Config change for the main loop.
    submitted: Option<Pending>,
    /// A change is submitted or being applied, its response is not sent yet.
    busy: bool,
    /// Last request except [`Request::Ping`] and [`Request::End`].
    last_request: Option<Instant>,
    /// Last request of the app session, `None` once it ended.
    last_seen: Option<Instant>,
}

impl ConfigService {
    /// Must be called before the server starts (advertising).
    /// `current` is the encoded active config, `input` / `output` are the HID config reports.
    pub fn register(
        width: u8,
        height: u8,
        current: Vec<u8>,
        wake: Arc<Notifier>,
        input: Arc<Mutex<BLECharacteristic>>,
        output: &Mutex<BLECharacteristic>,
    ) -> Self {
        let info = Info {
            protocol_version: hid::PROTOCOL_VERSION,
            config_version: CONFIG_VERSION,
            width,
            height,
            max_layouts: MAX_LAYOUTS as u8,
            max_combos: MAX_COMBOS as u8,
            firmware_version: env!("CARGO_PKG_VERSION").into(),
        };

        let config_service = Self {
            state: Arc::new(std::sync::Mutex::new(State {
                receiver: ConfigReceiver::new(width, height),
                current,
                submitted: None,
                busy: false,
                last_request: None,
                last_seen: None,
            })),
            input,
            wake,
            info: Arc::new(info),
        };

        let callback_service = config_service.clone();
        output.lock().on_write(move |args| {
            let mtu = args.desc().mtu() as usize;
            if mtu < hid::MIN_MTU {
                warn!("Config report over MTU {}, responses don't fit (needs {})", mtu, hid::MIN_MTU);
            }
            match hid::decode(args.recv_data()) {
                Some(data) => callback_service.on_request(data),
                None => {
                    warn!("Config report: bad length {:02x?}", args.recv_data());
                    callback_service.send(&Response::Error(ErrorCode::BadRequest));
                },
            }
        });

        config_service
    }

    /// Handles a request in the NimBLE task (the output report is locked, the input one is not).
    fn on_request(&self, data: &[u8]) {
        let (response, wake) = self.handle(data);
        if let Some(response) = response {
            self.send(&response);
        }
        if wake {
            self.wake_main_loop();
        }
    }

    /// Handles a request in the NimBLE task; the response is `None` if it's sent later by the main loop.
    /// Returns whether to wake the main loop up: session requests don't, so the keyboard can still idle.
    fn handle(&self, data: &[u8]) -> (Option<Response>, bool) {
        let Ok(request) = Request::from_bytes(data) else {
            warn!("Config service: bad request {:02x?}", data);
            return (Some(Response::Error(ErrorCode::BadRequest)), true);
        };

        let mut state = self.lock();
        match request {
            Request::Ping => {
                if state.last_seen.is_none() {
                    info!("Config session started");
                }
                state.last_seen = Some(Instant::now());
                return (Some(Response::Ok), false);
            },
            Request::End => {
                info!("Config session ended");
                state.last_seen = None;
                state.last_request = None;
                return (Some(Response::Ok), false);
            },
            _ => {},
        }
        if state.last_seen.is_none() {
            info!("Config session started");
        }
        state.last_request = Some(Instant::now());
        state.last_seen = state.last_request;
        (self.handle_transfer(&mut state, request), true)
    }

    /// Handles a config request, see [`Self::handle`].
    fn handle_transfer(&self, state: &mut State, request: Request) -> Option<Response> {
        if state.busy && request != Request::GetInfo {
            return Some(Response::Error(ErrorCode::Busy));
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
                    state.submitted = Some(Pending::Apply(config));
                    state.busy = true;
                    return None;
                },
                Err(code) => Response::Error(code),
            },
            Request::ResetToDefault => {
                info!("Config reset requested");
                state.receiver.abort();
                state.submitted = Some(Pending::Reset);
                state.busy = true;
                return None;
            },
            Request::Ping | Request::End => unreachable!("handled by the caller"),
        };

        if let Response::Error(code) = &response {
            warn!("Config request failed: {}", code);
        }
        Some(response)
    }

    /// Config change submitted by the app, to be applied by the main loop and answered with [`Self::finish`].
    pub fn take_submitted(&self) -> Option<Pending> {
        self.lock().submitted.take()
    }

    /// A config request came recently, the host is likely in the middle of a transfer.
    pub fn transfer_active(&self) -> bool {
        self.lock()
            .last_request
            .is_some_and(|t| t.elapsed() < TRANSFER_TIMEOUT)
    }

    /// The config app is connected: it pinged recently and didn't end the session.
    pub fn session_active(&self) -> bool {
        self.lock()
            .last_seen
            .is_some_and(|t| t.elapsed() < hid::SESSION_TIMEOUT)
    }

    /// Replaces the encoded active config, after any config change.
    pub fn set_current(&self, current: Vec<u8>) {
        self.lock().current = current;
    }

    /// Answers the change taken with [`Self::take_submitted`].
    pub fn finish(&self, response: Response) {
        self.lock().busy = false;
        self.send(&response);
    }

    /// Drops an unfinished transfer, a submitted change is still applied.
    pub fn on_disconnect(&self) {
        let mut state = self.lock();
        if state.receiver.is_active() {
            info!("Config transfer aborted");
        }
        state.receiver.abort();
        state.last_request = None;
        state.last_seen = None;
    }

    fn send(&self, response: &Response) {
        let bytes = response.to_bytes();
        // Fits a report: chunks are sized by it, other responses are small.
        let Some(report) = hid::encode(&bytes) else {
            warn!("Config response of {} bytes doesn't fit a report", bytes.len());
            return;
        };
        let mut input = self.input.lock();
        if input.subscribed_count() == 0 {
            warn!("Config response dropped, nobody is subscribed");
            return;
        }
        input.set_value(&report).notify();
    }

    fn wake_main_loop(&self) {
        // The main task lives as long as the program.
        unsafe {
            self.wake.notify(NonZeroU32::new(1).unwrap());
        }
    }

    /// The state stays consistent if a holder panicked: every update is a single assignment.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
