//! BLE config service, see [`protocol::ble`] for the GATT layout and the message flows.
//!
//! GATT callbacks run in the NimBLE host task and never touch [`Services`](crate::services::Services):
//! they only use [`State`] behind a mutex. Requests that change the config ([`Request::WriteCommit`],
//! [`Request::ResetToDefault`]) are handed to the main loop through [`ConfigService::take_submitted`],
//! the main loop applies them once all keys are up and answers with [`ConfigService::finish`].

use std::num::NonZeroU32;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

use esp32_nimble::utilities::{mutex::Mutex, BleUuid};
use esp32_nimble::{BLECharacteristic, BLEServer, NimbleProperties};
use esp_idf_svc::hal::task::notification::Notifier;
use log::{info, warn};
use protocol::ble::{self, chunk_payload_size, ConfigReceiver, ErrorCode, Info, Request, Response};
use protocol::config::{CONFIG_VERSION, MAX_COMBOS, MAX_LAYOUTS};

use crate::config::Pending;

/// The keyboard stays active (no idle connection parameters) this long after the last transfer request.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared between the GATT callbacks and the main loop.
#[derive(Clone)]
pub struct ConfigService {
    state: Arc<std::sync::Mutex<State>>,
    response: Arc<Mutex<BLECharacteristic>>,
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
    /// `current` is the encoded active config.
    pub fn register(server: &mut BLEServer, width: u8, height: u8, current: Vec<u8>, wake: Arc<Notifier>) -> Self {
        let info = Info {
            protocol_version: ble::PROTOCOL_VERSION,
            config_version: CONFIG_VERSION,
            width,
            height,
            max_layouts: MAX_LAYOUTS as u8,
            max_combos: MAX_COMBOS as u8,
            firmware_version: env!("CARGO_PKG_VERSION").into(),
        };

        let service = server.create_service(uuid(ble::SERVICE_UUID));
        let mut service = service.lock();

        let info_chr = service.create_characteristic(
            uuid(ble::INFO_UUID),
            NimbleProperties::READ | NimbleProperties::READ_ENC,
        );
        info_chr.lock().set_value(&info.to_bytes());

        let control = service.create_characteristic(
            uuid(ble::CONTROL_UUID),
            NimbleProperties::WRITE | NimbleProperties::WRITE_ENC,
        );

        // Responses are only sent for requests, which need encryption, so the CCCD doesn't.
        let response = service.create_characteristic(uuid(ble::RESPONSE_UUID), NimbleProperties::NOTIFY);

        let config_service = Self {
            state: Arc::new(std::sync::Mutex::new(State {
                receiver: ConfigReceiver::new(width, height),
                current,
                submitted: None,
                busy: false,
                last_request: None,
                last_seen: None,
            })),
            response,
            wake,
            info: Arc::new(info),
        };

        let callback_service = config_service.clone();
        control.lock().on_write(move |args| {
            // The control characteristic is locked here, the response one is not.
            let mtu = args.desc().mtu() as usize;
            let (response, wake) = callback_service.handle(args.recv_data(), mtu);
            if let Some(response) = response {
                callback_service.send(&response);
            }
            if wake {
                callback_service.wake_main_loop();
            }
        });

        config_service
    }

    /// Handles a request in the NimBLE task; the response is `None` if it's sent later by the main loop.
    /// Returns whether to wake the main loop up: session requests don't, so the keyboard can still idle.
    fn handle(&self, data: &[u8], mtu: usize) -> (Option<Response>, bool) {
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
        (self.handle_transfer(&mut state, request, mtu), true)
    }

    /// Handles a config request, see [`Self::handle`].
    fn handle_transfer(&self, state: &mut State, request: Request, mtu: usize) -> Option<Response> {
        if state.busy && request != Request::GetInfo {
            return Some(Response::Error(ErrorCode::Busy));
        }

        let response = match request {
            Request::GetInfo => Response::Info((*self.info).clone()),
            Request::ReadConfig { offset } => {
                Response::config_chunk(&state.current, offset, chunk_payload_size(mtu))
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
                info!("Config reset requested over BLE");
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

    /// Config change submitted over BLE, to be applied by the main loop and answered with [`Self::finish`].
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
            .is_some_and(|t| t.elapsed() < ble::SESSION_TIMEOUT)
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
        let mut characteristic = self.response.lock();
        if characteristic.subscribed_count() == 0 {
            warn!("Config response dropped, nobody is subscribed");
            return;
        }
        // Fits a notification: chunks are sized by the MTU, other responses are small.
        characteristic.set_value(&bytes).notify();
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

fn uuid(uuid: u128) -> BleUuid {
    BleUuid::from_uuid128(uuid.to_le_bytes())
}
