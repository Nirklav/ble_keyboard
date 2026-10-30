use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::bail;
use esp32_nimble::enums::{AuthReq, OwnAddrType, SecurityIOCap};
use esp32_nimble::{BLEDevice, BLEHIDDevice};
use esp32_nimble::{utilities::mutex::Mutex, BLECharacteristic, BLEServer};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use protocol::config::KeyAction;
use protocol::motion::Step;

use crate::bt::advertising::{advertise, set_profile_name};
use crate::bt::conn_params::ConnParams;
use crate::bt::gatt_cache::{gatt_change_delivered, signal_gatt_changed};
use crate::bt::link::{disconnect, local_address, Link};
use crate::bt::profiles::{self, lock, ProfileState, Profiles, Verdict};
use crate::bt::reports::{Report, Reports, CONFIG_ID, HID_REPORT_DESCRIPTOR, KEYBOARD_ID, MEDIA_KEYS_ID, MOUSE_ID};
use crate::storage::IRK_FILE;

/// Time for the released keys to reach the old host before a profile switch disconnects it:
/// a few connection intervals, see [`ConnParams`].
const RELEASE_DELAY: Duration = Duration::from_millis(50);

/// Stack of the [`Link`] thread: logging, NimBLE calls and the profiles.
const LINK_STACK_SIZE: usize = 6 * 1024;

/// ATT MTU asked for, a 247 bytes MTU fills one data length extended LL packet.
const PREFERRED_MTU: u16 = 247;

pub struct BleMouseKeyboard {
    server: &'static mut BLEServer,
    
    input_keyboard: Arc<Mutex<BLECharacteristic>>,
    input_media_keys: Arc<Mutex<BLECharacteristic>>,
    input_mouse: Arc<Mutex<BLECharacteristic>>,

    /// The config collection, see [`Self::config_reports`].
    input_config: Arc<Mutex<BLECharacteristic>>,
    output_config: Arc<Mutex<BLECharacteristic>>,
    profiles: Arc<std::sync::Mutex<Profiles>>,
    /// The connection is being dropped (a profile switch): nothing more goes to that host, another switch
    /// only changes the profile. Cleared by the disconnect callback, which applies the profile active then.
    dropping: Arc<AtomicBool>,
    /// The connected host is encrypted, bonded and accepted by the active profile (see
    /// [`Profiles::on_encrypted`]): only then it gets the reports. NimBLE lets any peer subscribe to the
    /// HID input reports (the CCCDs don't require encryption) and notifies them regardless.
    /// Set with [`Link::accepted`].
    accepted: Arc<AtomicBool>,
    link: Arc<Link>,
    /// Runs [`Link::run`], stopped before NimBLE is.
    link_thread: Option<JoinHandle<()>>,

    reports: Reports,
}

/// The config collection of the HID device ([`protocol::hid`]): the app writes the requests to the output
/// report, the keyboard answers in the input one.
pub struct ConfigReports {
    pub input: Arc<Mutex<BLECharacteristic>>,
    pub output: Arc<Mutex<BLECharacteristic>>,
}

impl BleMouseKeyboard {
    /// Sets the HID device up, the BLE profiles are saved in `nvs`. Nothing connects until [`Self::start`]: the
    /// services of the [`Self::config_reports`] are registered before.
    pub fn new(nvs: EspDefaultNvsPartition) -> anyhow::Result<Self> {
        unsafe {
            esp_idf_sys::ble_hs_cfg.store_gen_key_cb = Some(Self::gen_key_callback);
        }

        BLEDevice::init();

        let device = BLEDevice::take();
    
        // Static random address of the active profile, see `profiles`.
        device.set_own_addr_type(OwnAddrType::Random);
        // No IO, so pairing is always Just Works (unauthenticated): no MITM, asking for it makes some
        // hosts (Android) pair again on every connection. No passkey either.
        device
            .security()
            .set_auth(AuthReq::Bond | AuthReq::Sc)
            .set_io_cap(SecurityIOCap::NoInputNoOutput)
            .resolve_rpa();
        profiles::guard_pairing();

        if let Err(e) = device.set_preferred_mtu(PREFERRED_MTU) {
            log::warn!("Failed to set preferred MTU: {:?}", e);
        }

        let profiles = Arc::new(std::sync::Mutex::new(Profiles::load(nvs)));
        lock(&profiles).apply_address();

        let server = device.get_server();
        // Advertising is started by the disconnect callback, on the address of the profile active then.
        server.advertise_on_disconnect(false);
        let accepted = Arc::new(AtomicBool::new(false));
        let link = Arc::new(Link::new());
        let connect_accepted = accepted.clone();
        let connect_profiles = profiles.clone();
        let connect_link = link.clone();
        server.on_connect(move |server, desc| {
            info!("Client connected: {:?}", desc);
            let handle = desc.conn_handle();
            // NimBLE reports the connection after reading the remote features, a bonded host may have encrypted
            // it by then: `on_authentication_complete` has checked it already.
            let encrypted = desc.encrypted();
            // Also for a connection dropped below, in case dropping it fails.
            connect_link.connected(handle, &connect_accepted, encrypted);

            // Already stopped if the connection came from advertising.
            if let Err(e) = BLEDevice::take().get_advertising().lock().stop() {
                if e.code() != esp_idf_sys::BLE_HS_EALREADY {
                    log::warn!("Failed to stop advertising: {:?}", e);
                }
            }

            let peer: esp_idf_sys::ble_addr_t = desc.id_address().into();
            let reject = {
                let profiles = lock(&connect_profiles);
                // Advertising may have run on the address of a previous profile (setting the new one failed,
                // or a host connected while switching): that address is dropped, the next one is tried then.
                let active = local_address(handle).is_some_and(|local| profiles.is_active_address(&local));
                if !active {
                    info!("Connected on the address of a previous profile, disconnecting");
                }
                !active || profiles.bound_elsewhere(&peer)
            };
            if reject {
                // A host bound to another profile would lose that bond by pairing here.
                profiles::refuse_pairing(Some(&peer));
                disconnect(server, handle);
                return;
            }
            profiles::refuse_pairing(None);

            // No connection params until accepted: a pending connection update blocks the encryption start
            // (one LL control procedure at a time), and Windows answers it only after pairing.

            if encrypted {
                return;
            }
            // Asks the host to encrypt at once (a bonded one) or to pair, instead of waiting until it
            // touches an encrypted characteristic: it gets no reports until then, see `accepted`.
            let rc = unsafe { esp_idf_sys::ble_gap_security_initiate(desc.conn_handle()) };
            if rc != 0 && rc != esp_idf_sys::BLE_HS_EALREADY as i32 {
                log::warn!("Failed to request security: {}", rc);
            }
        });
        let auth_profiles = profiles.clone();
        let auth_accepted = accepted.clone();
        let auth_link = link.clone();
        server.on_authentication_complete(move |server, desc, result| {
            info!("Authentication complete: {:?}", result);
            // Not encrypted, it gets nothing (not accepted).
            if result.is_err() {
                return;
            }
            let peer: esp_idf_sys::ble_addr_t = desc.id_address().into();
            let Some(local) = local_address(desc.conn_handle()) else {
                return;
            };
            let verdict = lock(&auth_profiles).on_encrypted(&peer, &local, desc.bonded());
            match verdict {
                Verdict::Accept => {
                    auth_link.accepted(&auth_accepted);
                    auth_link.request_host_conn_params(desc.conn_handle());
                    gatt_change_delivered();
                }
                Verdict::Disconnect => disconnect(server, desc.conn_handle()),
                // Disconnected by removing its bond.
                Verdict::Reject => {}
            }
        });

        let mut hid = BLEHIDDevice::new(server);
        let input_keyboard = hid.input_report(KEYBOARD_ID);
        // The LED report, nothing reads it.
        hid.output_report(KEYBOARD_ID);
        let input_media_keys = hid.input_report(MEDIA_KEYS_ID);
        let input_mouse = hid.input_report(MOUSE_ID);
        let input_config = hid.input_report(CONFIG_ID);
        let output_config = hid.output_report(CONFIG_ID);

        hid.manufacturer("Nirklav");
        // Vendor ID source 0x02: a USB vendor ID.
        hid.pnp(0x02, protocol::hid::VENDOR_ID, protocol::hid::PRODUCT_ID, 0x0100);
        hid.hid_info(0x00, 0x01);

        hid.report_map(HID_REPORT_DESCRIPTOR);

        hid.set_battery_level(100);

        Ok(Self {
            server,
            input_keyboard,
            input_media_keys,
            input_mouse,
            input_config,
            output_config,
            profiles,
            dropping: Arc::new(AtomicBool::new(false)),
            accepted,
            link,
            link_thread: None,
            reports: Reports::default(),
        })
    }

    /// The config collection, for the config protocol to register its handlers on before [`Self::start`].
    pub fn config_reports(&self) -> ConfigReports {
        ConfigReports { input: self.input_config.clone(), output: self.output_config.clone() }
    }

    /// Starts the server and advertising; `on_disconnect` is called when a host disconnects.
    pub fn start(&mut self, on_disconnect: impl Fn() + Send + Sync + 'static) -> anyhow::Result<()> {
        let disconnect_profiles = self.profiles.clone();
        let disconnect_dropping = self.dropping.clone();
        let disconnect_accepted = self.accepted.clone();
        let disconnect_link = self.link.clone();
        self.server.on_disconnect(move |_desc, result| {
            info!("Client disconnected: {:?}", result);
            on_disconnect();
            disconnect_link.disconnected(&disconnect_accepted);
            profiles::refuse_pairing(None);
            // Cleared first: a switch from now on restarts advertising itself, an earlier one is applied here.
            disconnect_dropping.store(false, Ordering::Release);
            // On the address of the profile active now: the last one selected while the connection was being
            // dropped. If setting it fails, the previous address is advertised, a host connecting there is
            // disconnected and it's tried again.
            advertise(&disconnect_profiles, &disconnect_link);
        });

        let mut ble_advertising = BLEDevice::take().get_advertising().lock();
        ble_advertising.scan_response(false);
        set_profile_name(&mut ble_advertising, lock(&self.profiles).state().index)?;

        if let Err(e) = ble_advertising.start() {
            bail!("error on advertising start [code: {:?}]", e);
        }
        // The GATT server is started with the advertising.
        signal_gatt_changed();

        let (link, profiles, accepted) = (self.link.clone(), self.profiles.clone(), self.accepted.clone());
        let link_thread = std::thread::Builder::new()
            .name("ble_link".into())
            .stack_size(LINK_STACK_SIZE)
            .spawn(move || link.run(&profiles, &accepted))?;
        self.link_thread = Some(link_thread);
        Ok(())
    }

    unsafe extern "C" fn gen_key_callback(t: u8, key: *mut esp_idf_sys::ble_store_gen_key, _handle: u16) -> i32 {
        if t as u32 != esp_idf_sys::BLE_STORE_GEN_KEY_IRK {
            return 0;
        }

        let mut buf = [0u8; 16];

        let read_existing = std::fs::read(IRK_FILE)
            .ok()
            .filter(|irk| irk.len() == 16);

        match read_existing {
            Some(irk) => {
                buf.copy_from_slice(&irk);
                info!("IRK read");
            }
            None => {
                esp_idf_sys::esp_fill_random(buf.as_mut_ptr() as *mut c_void, 16);
                if let Err(e) = std::fs::write(IRK_FILE, &buf) {
                    log::error!("Failed to write IRK file: {}", e);
                    return esp_idf_sys::BLE_HS_ESTORE_FAIL as i32;
                }
                info!("IRK generated");
            }
        }

        (*key).__bindgen_anon_1.irk = buf;
        0
    }

    pub fn connected(&self) -> bool {
        self.server.connected_count() > 0
    }

    /// Connected to the host of the active profile, see [`Self::accepted`].
    pub fn host_connected(&self) -> bool {
        self.connected() && self.accepted.load(Ordering::Acquire) && !self.dropping.load(Ordering::Acquire)
    }

    /// Idle connection parameters (slave latency) for the host. Not for a connection that isn't accepted
    /// yet: it may be pairing, and a connection update would block its encryption (see `on_connect`).
    pub fn set_idle(&mut self) {
        self.link.set_idle(true);
        self.set_conn_params();
    }

    /// Active connection parameters for the host, see [`Self::set_idle`].
    pub fn set_active(&mut self) {
        self.link.set_idle(false);
        self.set_conn_params();
    }

    /// A firmware update is being received: the shortest interval while active ([`ConnParams::TRANSFER`]).
    pub fn set_transfer(&mut self, transfer: bool) {
        if self.link.set_transfer(transfer) {
            if let Some(desc) = self.server.connections().next() {
                let interval = ConnParams::interval_ms(desc.conn_handle()).unwrap_or(0.0);
                info!("Firmware transfer {}, connection interval {} ms", if transfer { "started" } else { "ended" }, interval);
            }
            self.set_conn_params();
        }
    }

    fn set_conn_params(&mut self) {
        // After the link has the state: a host accepted after it gets these params, see `on_authentication_complete`.
        if !self.host_connected() {
            return;
        }
        let params = self.link.main_params();
        if let Some(desc) = self.server.connections().next() {
            self.link.request_conn_params(desc.conn_handle(), params);
        }
    }

    /// Adds a held key to the reports, see [`Reports::press`].
    pub fn press(&mut self, action: &KeyAction) {
        self.reports.press(action);
    }

    /// Moves the mouse cursor and wheel in the reports.
    pub fn travel(&mut self, step: Step) {
        self.reports.travel(step);
    }

    /// Sends the reports to an accepted host, see [`Reports::flush`].
    pub fn flush(&mut self) {
        let send = self.host_connected().then_some(|report: Report, bytes: &[u8]| {
            let input = match report {
                Report::Keyboard => &self.input_keyboard,
                Report::MediaKeys => &self.input_media_keys,
                Report::Mouse => &self.input_mouse,
            };
            input.lock().set_value(bytes).notify();
        });
        self.reports.flush(send);
    }

    /// Active BLE profile.
    pub fn profile(&self) -> ProfileState {
        lock(&self.profiles).state()
    }

    /// Switches to profile `index` (see [`crate::bt::profiles`]), nothing if it's active already.
    pub fn select_profile(&mut self, index: u8) -> ProfileState {
        let selected = lock(&self.profiles).select(index);
        self.switched(selected)
    }

    /// Switches to the next profile (`step` 1) or the previous one (`step` -1).
    pub fn step_profile(&mut self, step: isize) -> ProfileState {
        let selected = lock(&self.profiles).step(step);
        self.switched(selected)
    }

    /// Forgets the host of the active profile, the profile waits for a new one.
    pub fn clear_profile(&mut self) -> ProfileState {
        let state = lock(&self.profiles).clear_active();
        self.reconnect();
        state
    }

    /// Forgets the hosts of all profiles.
    pub fn clear_all_profiles(&mut self) {
        lock(&self.profiles).clear_all();
        self.reconnect();
    }

    fn switched(&mut self, selected: Option<ProfileState>) -> ProfileState {
        match selected {
            Some(state) => {
                self.reconnect();
                state
            }
            None => self.profile(),
        }
    }

    /// Drops the current host, if any, and advertises on the address of the active profile.
    /// The host first gets all keys released, so nothing stays held there.
    fn reconnect(&mut self) {
        // Only an accepted host got reports, so only it gets released.
        let had_host = self.host_connected();
        self.release_all();
        // Already being dropped: the disconnect callback applies the profile selected meanwhile.
        if self.dropping.load(Ordering::Acquire) {
            return;
        }
        if self.connected() {
            if had_host {
                std::thread::sleep(RELEASE_DELAY);
            }
            // Set before the disconnect, the disconnect callback clears it. The host may have gone meanwhile.
            self.dropping.store(true, Ordering::Release);
            let handle = self.server.connections().next().map(|desc| desc.conn_handle());
            if let Some(handle) = handle {
                match self.server.disconnect(handle) {
                    Ok(()) => return,
                    // Being disconnected already (e.g. rejected), the disconnect callback comes.
                    Err(e) if e.code() == esp_idf_sys::BLE_HS_EALREADY => return,
                    // Gone meanwhile, its disconnect callback may have applied the old profile.
                    Err(e) if e.code() == esp_idf_sys::BLE_HS_ENOTCONN => {}
                    Err(e) => log::warn!("Failed to disconnect: {:?}", e),
                }
            }
            self.dropping.store(false, Ordering::Release);
        }
        // Unless connected: a connection advertises again from its disconnect callback.
        advertise(&self.profiles, &self.link);
    }

    /// Sends reports with nothing pressed; the keys still held are sent again by the next [`Self::flush`].
    fn release_all(&mut self) {
        self.reports.clear();
        self.flush();
    }
}

impl Drop for BleMouseKeyboard {
    fn drop(&mut self) {
        // The link thread uses NimBLE, it's stopped first.
        self.link.stop();
        if let Some(thread) = self.link_thread.take() {
            let _ = thread.join();
        }
        let _r = BLEDevice::deinit_full();
    }
}