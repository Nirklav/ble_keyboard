//! Owns the active BLE profile and connection lifecycle. A worker handles one-shot deadlines and
//! connection parameters.

use std::ffi::{c_int, c_void};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use esp32_nimble::{BLEConnDesc, BLEDevice};
use log::info;
use protocol::keymap::BtCommand;

use crate::bt::advertising::advertise;
use crate::bt::conn_params::ConnParams;
use crate::bt::connection::{Connection, ConnectionId};
use crate::bt::profiles::{self, ProfileState, Profiles, Verdict};
use crate::sleep_admission::{HostState, SleepReservation};

const DISCONNECT_RETRY: Duration = Duration::from_secs(1);

/// First wait before advertising is tried again after starting it failed, doubled on every failure
/// up to [`ADVERTISING_RETRY_MAX`].
const ADVERTISING_RETRY_MIN: Duration = Duration::from_secs(1);
const ADVERTISING_RETRY_MAX: Duration = Duration::from_secs(32);

/// Wait before connection params are sent again when NimBLE's request slot wasn't free yet.
const CONN_PARAMS_RETRY: Duration = Duration::from_millis(100);

/// Deadlines of [`Link`].
#[derive(Default)]
struct Deadlines {
    connection: Connection,
    /// Starting advertising failed, it's tried again then.
    advertise: Option<Instant>,
    /// Wait before the next advertising attempt, zero while advertising starts.
    advertise_backoff: Duration,
    /// Connection params to ask the host of a connection for, sent once no request is pending: only the
    /// latest ones, see [`Self::queue_conn_params`].
    conn_params: Option<ConnParams>,
    /// The main loop is idle, see [`Link::set_idle`].
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
    fn ready_handle(&self) -> Option<u16> {
        self.connection.ready().map(|id| id.handle)
    }

    fn next(&self) -> Option<Instant> {
        [self.connection.disconnect_at(), self.advertise, self.conn_params_retry]
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

    fn queue_conn_params(&mut self) {
        self.conn_params = self.ready_handle().map(|_| ConnParams::of(self.idle, self.transfer));
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

/// The profile lock serializes CONNECT and authentication with advertising and profile edits. The
/// deadlines lock arbitrates authorization, timeout, and report submission. Acquire advertising
/// before profiles, then deadlines when needed. Flash writes hold only the profile lock.
pub(super) struct Link {
    profiles: Mutex<Profiles>,
    deadlines: Mutex<Deadlines>,
    changed: Condvar,
}

impl Link {
    pub(super) fn new(mut profiles: Profiles) -> Self {
        profiles.apply_address();
        Self { profiles: Mutex::new(profiles), deadlines: Mutex::new(Deadlines::default()), changed: Condvar::new() }
    }

    fn profiles(&self) -> MutexGuard<'_, Profiles> {
        self.profiles.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn profile(&self) -> ProfileState {
        self.profiles().state()
    }

    /// Applies the active profile address after the advertising adapter stops advertising.
    pub(super) fn apply_profile_address(&self) -> u8 {
        let mut profiles = self.profiles();
        profiles.apply_address();
        profiles.state().index
    }

    fn deadlines(&self) -> MutexGuard<'_, Deadlines> {
        self.deadlines.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn has_host(&self) -> bool {
        self.deadlines().connection.id().is_some()
    }

    pub(super) fn ready_connection(&self, handle: u16) -> Option<ConnectionId> {
        self.deadlines().connection.ready().filter(|id| id.handle == handle)
    }

    pub(super) fn send_to<T>(&self, id: ConnectionId, send: impl FnOnce(u16) -> T) -> Option<T> {
        let deadlines = self.deadlines();
        // The NimBLE host task removes the old handle and delivers DISCONNECT before it can
        // install a replacement. While DISCONNECT waits for this lock, the handle can only
        // refer to the original connection or be absent.
        deadlines.connection.ready_handle_for(id).map(send)
    }

    pub(super) fn with_ready_host(&self, f: impl FnOnce(Option<u16>)) {
        let deadlines = self.deadlines();
        f(deadlines.ready_handle());
    }

    /// Registers the physical connection, then asks for security if encryption has yet to start.
    /// Returns whether a host was already bonded and became ready here.
    pub(super) fn connected(&self, desc: &BLEConnDesc, sleep: &SleepReservation<'_>) -> bool {
        let handle = desc.conn_handle();
        let peer = desc.id_address().into();
        let advertising = BLEDevice::take().get_advertising().lock();
        let mut profiles = self.profiles();
        let local = local_address(handle);
        // An advertising restart may have failed to apply the active profile's address.
        let active = local.is_some_and(|local| profiles.is_active_address(&local));
        if !active {
            info!("Connected on the address of a previous profile, disconnecting");
        }
        let valid = active && !profiles.bound_elsewhere(&peer);
        // A host bound elsewhere would lose that bond by pairing here.
        profiles::refuse_pairing((!valid).then_some(&peer));
        let mut deadlines = self.deadlines();
        let pairing_until = deadlines.connection.connected(handle, valid, Instant::now());
        sleep.set_host_state(pairing_until.map_or(HostState::Unavailable, HostState::PairingUntil));
        deadlines.conn_params = None;
        deadlines.conn_params_retry = None;
        // Advertising has stopped, the disconnect starts it again.
        deadlines.advertise = None;
        deadlines.advertise_backoff = Duration::ZERO;
        self.changed.notify_one();
        drop(deadlines);
        drop(advertising);

        let Some(local) = local.filter(|_| valid) else {
            return false;
        };
        if desc.encrypted() {
            // ENC_CHANGE can precede CONNECT. A fresh pairing can also be encrypted while
            // exchanging keys. In that case wait for the authentication-complete callback.
            return desc.bonded() && self.authorize(&mut profiles, handle, &peer, &local, true, sleep);
        }
        // A pending connection update would block starting encryption on Windows. Queue
        // connection params only after the profile accepts this host.
        let rc = unsafe { esp_idf_sys::ble_gap_security_initiate(handle) };
        if rc != 0 && rc != esp_idf_sys::BLE_HS_EALREADY as i32 {
            log::warn!("Failed to request security: {}", rc);
        }
        false
    }

    pub(super) fn authenticated(&self, desc: &BLEConnDesc, sleep: &SleepReservation<'_>) -> bool {
        let handle = desc.conn_handle();
        let Some(local) = local_address(handle) else {
            return false;
        };
        self.authorize(&mut self.profiles(), handle, &desc.id_address().into(), &local, desc.bonded(), sleep)
    }

    /// Claims authorization once, before [`Profiles::on_encrypted`] can change the bond or NVS.
    fn authorize(
        &self,
        profiles: &mut Profiles,
        handle: u16,
        peer: &esp_idf_sys::ble_addr_t,
        local: &esp_idf_sys::ble_addr_t,
        bonded: bool,
        sleep: &SleepReservation<'_>,
    ) -> bool {
        let mut deadlines = self.deadlines();
        // An early ENC_CHANGE has no validated CONNECT yet.
        let Some(claim) = deadlines.connection.claim_authentication(handle, Instant::now()) else {
            return false;
        };
        // The callback reservation now covers authorization, even past the pairing deadline.
        sleep.set_host_state(HostState::Unavailable);
        drop(deadlines);
        let accepted = matches!(profiles.on_encrypted(peer, local, bonded), Verdict::Accept);
        let mut deadlines = self.deadlines();
        let accepted = deadlines.connection.finish_authentication(claim, accepted, Instant::now());
        if accepted {
            sleep.set_host_state(HostState::Ready);
            deadlines.queue_conn_params();
        }
        self.changed.notify_one();
        accepted
    }

    pub(super) fn disconnected(&self, handle: u16, sleep: &SleepReservation<'_>) -> Option<ConnectionId> {
        let mut deadlines = self.deadlines();
        let id = deadlines.connection.disconnected(handle)?;
        sleep.set_host_state(HostState::Unavailable);
        profiles::refuse_pairing(None);
        // A pending request ends by itself, NimBLE calls `conn_params_done`.
        deadlines.conn_params = None;
        deadlines.conn_params_retry = None;
        self.changed.notify_one();
        Some(id)
    }

    pub(super) fn change_profile(
        &self,
        command: BtCommand,
        sleep: &SleepReservation<'_>,
        release: impl FnOnce(Option<u16>),
    ) -> ProfileState {
        let mut profiles = self.profiles();
        let mut restart_advertising = false;
        let state = profiles.change(command, || restart_advertising = !self.drop_host(sleep, release));
        drop(profiles);
        if restart_advertising {
            advertise(self);
        }
        state
    }

    /// Release held reports and invalidate the connection before changing the stored profile.
    /// Returns whether a connection exists and its disconnect will restart advertising.
    fn drop_host(&self, sleep: &SleepReservation<'_>, release: impl FnOnce(Option<u16>)) -> bool {
        let mut deadlines = self.deadlines();
        release(deadlines.ready_handle());
        let dropping = deadlines.connection.drop_host(Instant::now());
        sleep.set_host_state(HostState::Unavailable);
        deadlines.conn_params = None;
        deadlines.conn_params_retry = None;
        self.changed.notify_one();
        dropping
    }

    /// The main loop went idle or active, see [`BleMouseKeyboard::set_idle`](crate::bt::ble_mouse_keyboard::BleMouseKeyboard::set_idle).
    pub(super) fn set_idle(&self, idle: bool) {
        let mut deadlines = self.deadlines();
        deadlines.idle = idle;
        deadlines.queue_conn_params();
        self.changed.notify_one();
    }

    /// Returns the ready handle when the transfer state changed.
    pub(super) fn set_transfer(&self, transfer: bool) -> Option<u16> {
        let mut deadlines = self.deadlines();
        if deadlines.transfer == transfer {
            return None;
        }
        deadlines.transfer = transfer;
        deadlines.queue_conn_params();
        self.changed.notify_one();
        deadlines.ready_handle()
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

    pub(super) fn run(&self) {
        let mut deadlines = self.deadlines();
        while !deadlines.stop {
            let now = Instant::now();
            if let Some((id, pairing_timed_out)) = deadlines.connection.begin_disconnect(now) {
                if pairing_timed_out {
                    info!("Host not accepted before pairing timeout, disconnecting");
                }
                // NimBLE delivers DISCONNECT before processing another connection. Holding this lock
                // through terminate keeps that callback from passing the deadline on to a reused handle.
                let rc = unsafe {
                    esp_idf_sys::ble_gap_terminate(id.handle, esp_idf_sys::ble_error_codes_BLE_ERR_REM_USER_CONN_TERM as _)
                };
                match rc as u32 {
                    // NimBLE removes a handle just before DISCONNECT. Keep its identity for
                    // that callback even when the handle is already gone.
                    0 | esp_idf_sys::BLE_HS_EALREADY | esp_idf_sys::BLE_HS_ENOTCONN => {}
                    _ => {
                        log::warn!("Failed to disconnect: {}", rc);
                        deadlines.connection.retry_disconnect(id, Instant::now() + DISCONNECT_RETRY);
                    }
                }
                continue;
            }
            if deadlines.advertise.is_some_and(|deadline| deadline <= now) {
                deadlines.advertise = None;
                // Not under the lock: the callbacks set the deadlines while advertising is being started.
                drop(deadlines);
                advertise(self);
                deadlines = self.deadlines();
                continue;
            }
            if deadlines.conn_params_due(now) {
                let (Some(handle), Some(params)) = (deadlines.ready_handle(), deadlines.conn_params.take()) else {
                    continue;
                };
                deadlines.conn_params_retry = None;
                // The completion runs on the NimBLE task, outside this request. Keep the handle current
                // through submission, so a new connection never receives the old host's request.
                match self.send_conn_params(handle, params) {
                    ConnParamsSent::Pending => deadlines.conn_params_pending = true,
                    ConnParamsSent::Done => {}
                    ConnParamsSent::Busy => {
                        deadlines.conn_params = Some(params);
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

/// Own address of connection `handle`, `None` if there's no such connection.
fn local_address(handle: u16) -> Option<esp_idf_sys::ble_addr_t> {
    let mut desc = esp_idf_sys::ble_gap_conn_desc::default();
    let rc = unsafe { esp_idf_sys::ble_gap_conn_find(handle, &mut desc) };
    (rc == 0).then_some(desc.our_ota_addr)
}
