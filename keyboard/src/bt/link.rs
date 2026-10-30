//! The connection to the host: one-shot timeouts and connection params requests, run by a thread of its own
//! ([`Link::run`]), so the NimBLE callbacks only set deadlines.

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, MutexGuard};
use std::time::{Duration, Instant};

use esp32_nimble::{BLEDevice, BLEServer};
use log::info;

use crate::bt::advertising::advertise;
use crate::bt::conn_params::ConnParams;
use crate::bt::profiles::Profiles;

/// A host not accepted this long after connecting is dropped, it would hold the only connection.
/// Pairing a new host takes a few seconds, Windows may take longer.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// First wait before advertising is tried again after starting it failed, doubled on every failure
/// up to [`ADVERTISING_RETRY_MAX`].
const ADVERTISING_RETRY_MIN: Duration = Duration::from_secs(1);
const ADVERTISING_RETRY_MAX: Duration = Duration::from_secs(32);

/// Wait before connection params are sent again when NimBLE's request slot wasn't free yet.
const CONN_PARAMS_RETRY: Duration = Duration::from_millis(100);

/// Deadlines of [`Link`].
#[derive(Default)]
struct Deadlines {
    /// Connection not accepted yet and when it's dropped: a host that doesn't encrypt or failed to pair
    /// would hold the only connection. Set on every connect, so a new connection always gets the full
    /// [`ACCEPT_TIMEOUT`]; cleared when it's accepted or gone.
    accept: Option<(u16, Instant)>,
    /// Starting advertising failed, it's tried again then.
    advertise: Option<Instant>,
    /// Wait before the next advertising attempt, zero while advertising starts.
    advertise_backoff: Duration,
    /// Connection params to ask the host of a connection for, sent once no request is pending: only the
    /// latest ones, see [`Link::request_conn_params`].
    conn_params: Option<(u16, ConnParams)>,
    /// The main loop is idle, see [`Link::request_host_conn_params`].
    idle: bool,
    /// A firmware update is being received, see [`BleMouseKeyboard::set_transfer`](crate::bt::ble_mouse_keyboard::BleMouseKeyboard::set_transfer).
    transfer: bool,
    /// A connection params request waits for the host, see [`ConnParams::request`].
    conn_params_pending: bool,
    /// NimBLE's request slot wasn't free yet, `conn_params` are sent then.
    conn_params_retry: Option<Instant>,
    stop: bool,
}

impl Deadlines {
    fn next(&self) -> Option<Instant> {
        [self.accept.map(|(_, deadline)| deadline), self.advertise, self.conn_params_retry]
            .into_iter()
            .flatten()
            .min()
    }

    /// `conn_params` can be sent now.
    fn conn_params_due(&self, now: Instant) -> bool {
        self.conn_params.is_some()
            && !self.conn_params_pending
            && self.conn_params_retry.is_none_or(|deadline| deadline <= now)
    }
}

/// What came of sending connection params, see [`Link::send_conn_params`].
enum ConnParamsSent {
    /// Waits for the host.
    Pending,
    /// In use already, the connection is gone, or the request failed.
    Done,
    /// NimBLE's request slot isn't free yet.
    Busy,
}

/// One-shot timeouts of the link, run by a thread of their own ([`Link::run`]) that sleeps until the next
/// deadline and without one doesn't wake up at all (light sleep). Not an esp_timer: its task has a small
/// stack and runs the NimBLE timers too, while this takes the advertising and profiles locks and waits for
/// HCI commands. The NimBLE callbacks only set the deadlines.
pub(super) struct Link {
    deadlines: std::sync::Mutex<Deadlines>,
    changed: Condvar,
}

impl Link {
    pub(super) fn new() -> Self {
        Self { deadlines: std::sync::Mutex::new(Deadlines::default()), changed: Condvar::new() }
    }

    fn deadlines(&self) -> MutexGuard<'_, Deadlines> {
        self.deadlines.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Connection `handle` has been made: not accepted, dropped after [`ACCEPT_TIMEOUT`] unless it is.
    /// An `encrypted` one has been checked before (see `on_connect`): accepted or being dropped already.
    pub(super) fn connected(&self, handle: u16, accepted: &AtomicBool, encrypted: bool) {
        let mut deadlines = self.deadlines();
        if !encrypted {
            accepted.store(false, Ordering::Release);
        }
        if !accepted.load(Ordering::Acquire) {
            deadlines.accept = Some((handle, Instant::now() + ACCEPT_TIMEOUT));
        }
        // An encrypted connection may have asked for its params already.
        if !encrypted {
            deadlines.conn_params = None;
        }
        // Advertising has stopped, the disconnect starts it again.
        deadlines.advertise = None;
        deadlines.advertise_backoff = Duration::ZERO;
        self.changed.notify_one();
    }

    /// The connection is accepted. Under the deadlines lock, so the accept timeout either sees it or is
    /// cancelled before it drops the connection.
    pub(super) fn accepted(&self, accepted: &AtomicBool) {
        let mut deadlines = self.deadlines();
        accepted.store(true, Ordering::Release);
        deadlines.accept = None;
        self.changed.notify_one();
    }

    pub(super) fn disconnected(&self, accepted: &AtomicBool) {
        let mut deadlines = self.deadlines();
        accepted.store(false, Ordering::Release);
        deadlines.accept = None;
        // A pending request ends by itself, NimBLE calls `conn_params_done`.
        deadlines.conn_params = None;
        deadlines.conn_params_retry = None;
        self.changed.notify_one();
    }

    /// Asks the host of connection `handle` for `params`. Sent by the link thread, one request at a time:
    /// params asked for while one is pending replace the ones that wait, only the latest are sent.
    pub(super) fn request_conn_params(&self, handle: u16, params: ConnParams) {
        let mut deadlines = self.deadlines();
        deadlines.conn_params = Some((handle, params));
        self.changed.notify_one();
    }

    /// The main loop went idle or active, see [`BleMouseKeyboard::set_idle`](crate::bt::ble_mouse_keyboard::BleMouseKeyboard::set_idle).
    pub(super) fn set_idle(&self, idle: bool) {
        self.deadlines().idle = idle;
    }

    /// See [`BleMouseKeyboard::set_transfer`](crate::bt::ble_mouse_keyboard::BleMouseKeyboard::set_transfer); returns whether it changed.
    pub(super) fn set_transfer(&self, transfer: bool) -> bool {
        let mut deadlines = self.deadlines();
        let changed = deadlines.transfer != transfer;
        deadlines.transfer = transfer;
        changed
    }

    /// Params of the main loop's state.
    pub(super) fn main_params(&self) -> ConnParams {
        let deadlines = self.deadlines();
        ConnParams::of(deadlines.idle, deadlines.transfer)
    }

    /// The host of connection `handle` has been accepted: asks it for the params of the main loop's state,
    /// the loop doesn't send them to a host accepted while it's idle.
    pub(super) fn request_host_conn_params(&self, handle: u16) {
        let mut deadlines = self.deadlines();
        let params = ConnParams::of(deadlines.idle, deadlines.transfer);
        deadlines.conn_params = Some((handle, params));
        self.changed.notify_one();
    }

    /// Sends `params` to the host of connection `handle`, unless they're in use already.
    fn send_conn_params(&self, handle: u16, params: ConnParams) -> ConnParamsSent {
        if params.in_use(handle) != Some(false) {
            return ConnParamsSent::Done;
        }
        // The link outlives NimBLE's callbacks: NimBLE is stopped before it's dropped.
        let rc = params.request(handle, conn_params_done, self as *const Link as *mut c_void);
        match rc as u32 {
            0 => ConnParamsSent::Pending,
            esp_idf_sys::BLE_HS_ENOMEM => ConnParamsSent::Busy,
            esp_idf_sys::BLE_HS_ENOTCONN => ConnParamsSent::Done,
            _ => {
                log::warn!("Failed to request connection params: {}", rc);
                ConnParamsSent::Done
            }
        }
    }

    pub(super) fn advertising_started(&self) {
        let mut deadlines = self.deadlines();
        deadlines.advertise = None;
        deadlines.advertise_backoff = Duration::ZERO;
        self.changed.notify_one();
    }

    /// Starting advertising failed: it's tried again after a wait, longer on every failure.
    pub(super) fn retry_advertising(&self) {
        let mut deadlines = self.deadlines();
        let backoff = (deadlines.advertise_backoff * 2).clamp(ADVERTISING_RETRY_MIN, ADVERTISING_RETRY_MAX);
        deadlines.advertise_backoff = backoff;
        deadlines.advertise = Some(Instant::now() + backoff);
        self.changed.notify_one();
    }

    pub(super) fn stop(&self) {
        self.deadlines().stop = true;
        self.changed.notify_one();
    }

    pub(super) fn run(&self, profiles: &std::sync::Mutex<Profiles>, accepted: &AtomicBool) {
        let mut deadlines = self.deadlines();
        while !deadlines.stop {
            let now = Instant::now();
            if let Some((handle, deadline)) = deadlines.accept {
                if deadline <= now {
                    deadlines.accept = None;
                    // Still under the lock, see `accepted`.
                    if !accepted.load(Ordering::Acquire) {
                        info!("Host not accepted in {:?}, disconnecting", ACCEPT_TIMEOUT);
                        disconnect(BLEDevice::take().get_server(), handle);
                    }
                    continue;
                }
            }
            if deadlines.advertise.is_some_and(|deadline| deadline <= now) {
                deadlines.advertise = None;
                // Not under the lock: the callbacks set the deadlines while advertising is being started.
                drop(deadlines);
                advertise(profiles, self);
                deadlines = self.deadlines();
                continue;
            }
            if deadlines.conn_params_due(now) {
                let Some((handle, params)) = deadlines.conn_params.take() else {
                    continue;
                };
                deadlines.conn_params_retry = None;
                deadlines.conn_params_pending = true;
                // Not under the lock: `conn_params_done` may run before the request returns.
                drop(deadlines);
                let sent = self.send_conn_params(handle, params);
                deadlines = self.deadlines();
                match sent {
                    ConnParamsSent::Pending => {}
                    ConnParamsSent::Done => deadlines.conn_params_pending = false,
                    ConnParamsSent::Busy => {
                        deadlines.conn_params_pending = false;
                        // Unless newer ones are asked for meanwhile. If the connection is gone, they're dropped
                        // when sent: no connection, nothing to send.
                        if deadlines.conn_params.is_none() {
                            deadlines.conn_params = Some((handle, params));
                        }
                        deadlines.conn_params_retry = Some(Instant::now() + CONN_PARAMS_RETRY);
                    }
                }
                continue;
            }
            deadlines = match deadlines.next() {
                Some(deadline) => {
                    let waited = self.changed.wait_timeout(deadlines, deadline - now);
                    waited.unwrap_or_else(|e| e.into_inner()).0
                }
                None => self.changed.wait(deadlines).unwrap_or_else(|e| e.into_inner()),
            };
        }
    }
}

/// A connection params request has ended (see [`ConnParams::request`]), `arg` is the [`Link`].
/// Runs in the NimBLE host task while the request still takes NimBLE's slot: the link thread, of a lower
/// priority, sends the next one after it's freed.
unsafe extern "C" fn conn_params_done(_handle: u16, status: c_int, arg: *mut c_void) {
    let link = &*(arg as *const Link);
    match status as u32 {
        0 => {}
        esp_idf_sys::BLE_HS_EREJECT => info!("Connection params rejected by the host"),
        esp_idf_sys::BLE_HS_ETIMEOUT => info!("Connection params not answered by the host"),
        // The connection is gone.
        _ => {}
    }
    let mut deadlines = link.deadlines();
    deadlines.conn_params_pending = false;
    link.changed.notify_one();
}

/// Disconnects connection `handle`, nothing if it's gone or going already.
pub(super) fn disconnect(server: &mut BLEServer, handle: u16) {
    match server.disconnect(handle) {
        Ok(()) => {}
        Err(e) if e.code() == esp_idf_sys::BLE_HS_EALREADY || e.code() == esp_idf_sys::BLE_HS_ENOTCONN => {}
        Err(e) => log::warn!("Failed to disconnect: {:?}", e),
    }
}

/// Own address of connection `handle`, `None` if there's no such connection.
pub(super) fn local_address(handle: u16) -> Option<esp_idf_sys::ble_addr_t> {
    let mut desc = esp_idf_sys::ble_gap_conn_desc::default();
    let rc = unsafe { esp_idf_sys::ble_gap_conn_find(handle, &mut desc) };
    (rc == 0).then_some(desc.our_ota_addr)
}
