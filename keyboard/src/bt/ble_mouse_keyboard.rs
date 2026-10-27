use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use esp32_nimble::{BLEAdvertisementData, BLEAdvertising, BLEHIDDevice, hid::*};
use esp32_nimble::utilities::BleUuid;
use esp32_nimble::{utilities::mutex::Mutex, BLECharacteristic, BLEServer};
use esp_idf_svc::hal::task::notification::Notifier;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use zerocopy::IntoBytes;

use crate::bt::config_service::ConfigService;
use crate::bt::conn_params::ConnParams;
use crate::bt::profiles::{self, ProfileState, Profiles, Verdict};

use super::*;

/// IRK sent to the hosts when they pair. The keyboard doesn't use resolvable addresses (see
/// [`crate::bt::profiles`]), so it only has to stay the same.
static IRK_FILE: &str = "/spiflash/irk_keym.txt";

/// Shown by the hosts with the profile number, see [`set_profile_name`].
const DEVICE_NAME: &str = "Nirklav Keyboard";

/// Human Interface Device service.
const HID_SERVICE_UUID: u16 = 0x1812;

/// Time for the released keys to reach the old host before a profile switch disconnects it:
/// a few connection intervals, see [`ConnParams`].
const RELEASE_DELAY: Duration = Duration::from_millis(50);

/// A host not accepted this long after connecting is dropped, it would hold the only connection.
/// Pairing a new host takes a few seconds, Windows may take longer.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// First wait before advertising is tried again after starting it failed, doubled on every failure
/// up to [`ADVERTISING_RETRY_MAX`].
const ADVERTISING_RETRY_MIN: Duration = Duration::from_secs(1);
const ADVERTISING_RETRY_MAX: Duration = Duration::from_secs(32);

/// Wait before connection params are sent again when NimBLE's request slot wasn't free yet.
const CONN_PARAMS_RETRY: Duration = Duration::from_millis(100);

/// Stack of the [`Link`] thread: logging, NimBLE calls and the profiles.
const LINK_STACK_SIZE: usize = 6 * 1024;

/// Last GATT database version the bonded hosts were told about, see [`signal_gatt_changed`].
static GATT_DB_FILE: &str = "/spiflash/gatt_db.txt";
/// Bump on any change of the services, characteristics or the HID report map:
/// 1 - HID only, 2 - config service, 3 - keys up to F24.
const GATT_DB_VERSION: &str = "3";

/// ATT MTU asked for, a 247 bytes MTU fills one data length extended LL packet.
const PREFERRED_MTU: u16 = 247;

const KEYBOARD_ID: u8 = 0x01;
const MEDIA_KEYS_ID: u8 = 0x02;
const MOUSE_ID: u8 = 0x03;

const HID_REPORT_DESCRIPTOR: &[u8] = hid!(
    (USAGE_PAGE, 0x01),         // Generic Desktop Ctrls
    (USAGE, 0x06),              // Keyboard
    (COLLECTION, 0x01),         // Application
    // ------------------------------------------------- Keyboard
    (REPORT_ID, KEYBOARD_ID),
    (USAGE_PAGE, 0x07),         // Kbrd/Keypad
    (USAGE_MINIMUM, 0xE0),
    (USAGE_MAXIMUM, 0xE7),
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x01),
    (REPORT_SIZE, 0x01),
    (REPORT_COUNT, 0x08),
    (HIDINPUT, 0x02),           // Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position
    (REPORT_COUNT, 0x01),       // 1 byte (Reserved)
    (REPORT_SIZE, 0x08),
    (HIDINPUT, 0x01),           // Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position
    (REPORT_COUNT, 0x05),       // 5 bits (Num lock, Caps lock, Scroll lock, Compose, Kana)
    (REPORT_SIZE, 0x01),
    (USAGE_PAGE, 0x08),         // LEDs
    (USAGE_MINIMUM, 0x01),      // Num Lock
    (USAGE_MAXIMUM, 0x05),      // Kana
    (HIDOUTPUT, 0x02),          // Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position,Non-volatile
    (REPORT_COUNT, 0x01),       // 3 bits (Padding)
    (REPORT_SIZE, 0x03),
    (HIDOUTPUT, 0x01),          // Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position,Non-volatile
    (REPORT_COUNT, 0x06),       // 6 bytes (Keys)
    (REPORT_SIZE, 0x08),
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x73),    // up to F24
    (USAGE_PAGE, 0x07),         // Kbrd/Keypad
    (USAGE_MINIMUM, 0x00),
    (USAGE_MAXIMUM, 0x73),
    (HIDINPUT, 0x00),           // Data,Array,Abs,No Wrap,Linear,Preferred State,No Null Position
    (END_COLLECTION),
    // ------------------------------------------------- Media Keys
    (USAGE_PAGE, 0x0C),         // Consumer
    (USAGE, 0x01),              // Consumer Control
    (COLLECTION, 0x01),         // Application
    (REPORT_ID, MEDIA_KEYS_ID),
    (USAGE_PAGE, 0x0C),         // Consumer
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x01),
    (REPORT_SIZE, 0x01),
    (REPORT_COUNT, 0x08),
    (USAGE, 0xB5),              // Scan Next Track; bit 0
    (USAGE, 0xB6),              // Scan Previous Track; bit 1
    (USAGE, 0xB7),              // Stop; bit 2
    (USAGE, 0xCD),              // Play/Pause; bit 3
    (USAGE, 0xE2),              // Mute; bit 4
    (USAGE, 0xE9),              // Volume Increment; bit 5
    (USAGE, 0xEA),              // Volume Decrement; bit 6
    (USAGE, 0x92, 0x01),        // Calculator; bit 7
    (HIDINPUT, 0x02),           // Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position
    (END_COLLECTION),
    // ------------------------------------------------- Mouse
    (USAGE_PAGE, 0x01),         // Generic Desktop
    (USAGE, 0x02),              // Mouse
    (COLLECTION, 0x01),         // Application
    (REPORT_ID, MOUSE_ID),
    (USAGE, 0x01),              // Pointer
    (COLLECTION, 0x00),         // Physical
    // ------------------------------------------------- Buttons (Left, Right, Middle, Back, Forward)
    (USAGE_PAGE, 0x09),         // Button
    (USAGE_MINIMUM, 0x01),      // Button 1
    (USAGE_MAXIMUM, 0x05),      // Button 5
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x01),
    (REPORT_SIZE, 0x01),
    (REPORT_COUNT, 0x05),
    (HIDINPUT, 0x02),           // Data, Variable, Absolute; 5 button bits
    // ------------------------------------------------- Padding
    (REPORT_SIZE, 0x03),
    (REPORT_COUNT, 0x01),
    (HIDINPUT, 0x03),           // Constant, Variable, Absolute; 3 bit padding
    // ------------------------------------------------- X/Y position, Wheel
    (USAGE_PAGE, 0x01),         // Generic Desktop
    (USAGE, 0x30),              // X
    (USAGE, 0x31),              // Y
    (USAGE, 0x38),              // Wheel
    (LOGICAL_MINIMUM, 0x81),    // -127
    (LOGICAL_MAXIMUM, 0x7f),
    (REPORT_SIZE, 0x08),
    (REPORT_COUNT, 0x03),
    (HIDINPUT, 0x06),           // Data, Variable, Relative; 3 bytes (X,Y,Wheel)
    // ------------------------------------------------- Horizontal wheel
    (USAGE_PAGE, 0x0c),         // Consumer Devices
    (USAGE, 0x38, 0x02),        // AC Pan
    (LOGICAL_MINIMUM, 0x81),    // -127
    (LOGICAL_MAXIMUM, 0x7f),
    (REPORT_SIZE, 0x08),
    (REPORT_COUNT, 0x01),
    (HIDINPUT, 0x06),           // Data, Var, Rel
    (END_COLLECTION),
    (END_COLLECTION)
);

#[allow(unused)]
pub struct BleMouseKeyboard {
    server: &'static mut BLEServer,
    
    input_keyboard: Arc<Mutex<BLECharacteristic>>,
    output_keyboard: Arc<Mutex<BLECharacteristic>>,
    input_media_keys: Arc<Mutex<BLECharacteristic>>,
    input_mouse: Arc<Mutex<BLECharacteristic>>,

    config_service: ConfigService,
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

    key_report: KeyReport,
    media_key_report: MediaKeyReport,
    mouse_report: MouseReport,

    sent_key_report: KeyReport,
    sent_media_key_report: MediaKeyReport,
    sent_mouse_buttons: u8,
}

#[allow(unused)]
impl BleMouseKeyboard {
    /// `width` x `height` keys, `current_config` is the encoded active config,
    /// `wake` wakes the main loop up on config requests. The BLE profiles are saved in `nvs`.
    pub fn new(
        width: u8,
        height: u8,
        current_config: Vec<u8>,
        wake: Arc<Notifier>,
        nvs: EspDefaultNvsPartition,
    ) -> anyhow::Result<Self> {
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

            {
                let mut ble_advertising = BLEDevice::take().get_advertising().lock();
                ble_advertising.stop();
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
        let output_keyboard = hid.output_report(KEYBOARD_ID);
        let input_media_keys = hid.input_report(MEDIA_KEYS_ID);
        let input_mouse = hid.input_report(MOUSE_ID);

        hid.manufacturer("Nirklav");
        hid.pnp(0x02, 0x0000, 0x0000, 0x0100);
        hid.hid_info(0x00, 0x01);

        hid.report_map(HID_REPORT_DESCRIPTOR);

        hid.set_battery_level(100);

        // After the HID services, so their handles stay the same as in older firmware.
        let config_service = ConfigService::register(server, width, height, current_config, wake);
        let disconnect_service = config_service.clone();
        let disconnect_profiles = profiles.clone();
        let dropping = Arc::new(AtomicBool::new(false));
        let disconnect_dropping = dropping.clone();
        let disconnect_accepted = accepted.clone();
        let disconnect_link = link.clone();
        server.on_disconnect(move |_desc, result| {
            info!("Client disconnected: {:?}", result);
            disconnect_service.on_disconnect();
            disconnect_link.disconnected(&disconnect_accepted);
            profiles::refuse_pairing(None);
            // Cleared first: a switch from now on restarts advertising itself, an earlier one is applied here.
            disconnect_dropping.store(false, Ordering::Release);
            // On the address of the profile active now: the last one selected while the connection was being
            // dropped. If setting it fails, the previous address is advertised, a host connecting there is
            // disconnected and it's tried again.
            advertise(&disconnect_profiles, &disconnect_link);
        });

        let mut ble_advertising = device.get_advertising().lock();
        ble_advertising.scan_response(false);
        set_profile_name(&mut ble_advertising, lock(&profiles).state().index)?;

        if let Err(e) = ble_advertising.start() {
            bail!("error on advertising start [code: {:?}]", e);
        }
        // The GATT server is started with the advertising.
        signal_gatt_changed();

        let link_thread = {
            let (link, profiles, accepted) = (link.clone(), profiles.clone(), accepted.clone());
            std::thread::Builder::new()
                .name("ble_link".into())
                .stack_size(LINK_STACK_SIZE)
                .spawn(move || link.run(&profiles, &accepted))?
        };

        Ok(Self {
            server,
            input_keyboard,
            output_keyboard,
            input_media_keys,
            input_mouse,
            config_service,
            profiles,
            dropping,
            accepted,
            link,
            link_thread: Some(link_thread),
            key_report: KeyReport::default(),
            media_key_report: MediaKeyReport::default(),
            mouse_report: MouseReport::default(),
            sent_key_report: KeyReport::default(),
            sent_media_key_report: MediaKeyReport::default(),
            sent_mouse_buttons: 0,
        })
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

    pub fn config_service(&self) -> &ConfigService {
        &self.config_service
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
    pub fn set_idle(&mut self) -> anyhow::Result<()> {
        // Before the check: a host accepted after it gets the idle params, see `on_authentication_complete`.
        self.link.set_idle(true);
        if !self.host_connected() {
            return Ok(());
        }
        self.set_conn_params(ConnParams::IDLE)
    }

    /// Active connection parameters for the host, see [`Self::set_idle`].
    pub fn set_active(&mut self) -> anyhow::Result<()>  {
        self.link.set_idle(false);
        if !self.host_connected() {
            return Ok(());
        }
        self.set_conn_params(ConnParams::ACTIVE)
    }

    fn set_conn_params(&mut self, params: ConnParams) -> anyhow::Result<()> {
        if let Some(desc) = self.server.connections().next() {
            self.link.request_conn_params(desc.conn_handle(), params);
        }
        Ok(())
    }

    pub fn press_key(&mut self, key: Key) {
        let keys = &mut self.key_report.keys;
        for i in 0..keys.len() {
            if keys[i] == 0 {
                keys[i] = key as u8;
                break;
            }
        }
    }

    pub fn press_key_mod(&mut self, modifier: KeyMod) {
        self.key_report.modifiers |= modifier.bits();
    }

    pub fn press_media_key(&mut self, key: MediaKey) {
        self.media_key_report.keys |= key.report_bit();
    }

    pub fn press_mouse(&mut self, key: MouseKey) {
        self.mouse_report.buttons |= key.bits();
    }

    pub fn travel_mouse_cursor(&mut self, x: i8, y: i8) {
        self.mouse_report.x = self.mouse_report.x.saturating_add(x);
        self.mouse_report.y = self.mouse_report.y.saturating_add(y);
    }

    pub fn travel_mouse_wheel(&mut self, v: i8) {
        self.mouse_report.wheel = self.mouse_report.wheel.saturating_add(v);
    }

    pub fn flush(&mut self) {
        if self.host_connected() {
            if self.key_report != self.sent_key_report {
                self.input_keyboard.lock().set_value(self.key_report.as_bytes()).notify();
            }

            if self.media_key_report != self.sent_media_key_report {
                self.input_media_keys.lock().set_value(self.media_key_report.as_bytes()).notify();
            }

            let mouse = &self.mouse_report;
            let moved = mouse.x != 0 || mouse.y != 0 || mouse.wheel != 0 || mouse.h_wheel != 0;
            if moved || mouse.buttons != self.sent_mouse_buttons {
                self.input_mouse.lock().set_value(self.mouse_report.as_bytes()).notify();
            }

            self.sent_key_report = self.key_report;
            self.sent_media_key_report = self.media_key_report;
            self.sent_mouse_buttons = self.mouse_report.buttons;
        } else {
            // A host connecting later has to receive the keys that are still held.
            self.sent_key_report = KeyReport::default();
            self.sent_media_key_report = MediaKeyReport::default();
            self.sent_mouse_buttons = 0;
        }

        self.key_report = KeyReport::default();
        self.media_key_report = MediaKeyReport::default();
        self.mouse_report = MouseReport::default();
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
    pub fn clear_all_profiles(&mut self) -> ProfileState {
        let state = lock(&self.profiles).clear_all();
        self.reconnect();
        state
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
        self.restart_advertising();
    }

    /// Advertises on the address of the active profile, unless connected: a connection advertises again
    /// from its disconnect callback.
    fn restart_advertising(&mut self) {
        advertise(&self.profiles, &self.link);
    }

    /// Sends reports with nothing pressed; the keys still held are sent again by the next [`Self::flush`].
    fn release_all(&mut self) {
        self.key_report = KeyReport::default();
        self.media_key_report = MediaKeyReport::default();
        self.mouse_report = MouseReport::default();
        self.flush();
    }
}

/// Advertises on the address of the active profile, unless connected. If starting fails, [`Link`]
/// tries again later.
fn advertise(profiles: &std::sync::Mutex<Profiles>, link: &Link) {
    if restart_advertising(profiles) {
        link.advertising_started();
    } else {
        link.retry_advertising();
    }
}

/// Advertises on the address of the active profile unless connected, a running advertising is restarted.
/// `false` if starting it failed.
fn restart_advertising(profiles: &std::sync::Mutex<Profiles>) -> bool {
    let mut advertising = BLEDevice::take().get_advertising().lock();
    // Checked under the advertising lock: a new connection stops advertising under it too.
    if BLEDevice::take().get_server().connected_count() > 0 {
        return true;
    }
    if advertising.is_advertising() {
        if let Err(e) = advertising.stop() {
            log::warn!("Failed to stop advertising: {:?}", e);
        }
    }
    {
        let mut profiles = lock(profiles);
        profiles.apply_address();
        if let Err(e) = set_profile_name(&mut advertising, profiles.state().index) {
            log::warn!("{:?}", e);
        }
    }
    match advertising.start() {
        Ok(()) => true,
        Err(e) if e.code() == esp_idf_sys::BLE_HS_EALREADY => true,
        Err(e) => {
            log::warn!("Failed to start advertising: {:?}", e);
            false
        }
    }
}

/// Sets the name of profile `index` as the device name and in the advertising data, "Nirklav Keyboard 1"
/// and so on: the hosts tell the profiles apart when pairing and show which one they are on.
/// Only while not advertising.
fn set_profile_name(advertising: &mut BLEAdvertising, index: u8) -> anyhow::Result<()> {
    let name = format!("{} {}", DEVICE_NAME, index + 1);
    if let Err(e) = BLEDevice::set_device_name(&name) {
        bail!("error on set device name [code: {:?}]", e);
    }

    let mut data = BLEAdvertisementData::new();
    // Flags, the name, the appearance and the HID service fill the 31 bytes exactly.
    data.name(&name)
        .appearance(0x03C1)
        .add_service_uuid(BleUuid::from_uuid16(HID_SERVICE_UUID));
    if let Err(e) = advertising.set_data(&mut data) {
        bail!("error on data advertising set [code: {:?}]", e);
    }
    Ok(())
}

/// Disconnects connection `handle`, nothing if it's gone or going already.
fn disconnect(server: &mut BLEServer, handle: u16) {
    match server.disconnect(handle) {
        Ok(()) => {}
        Err(e) if e.code() == esp_idf_sys::BLE_HS_EALREADY || e.code() == esp_idf_sys::BLE_HS_ENOTCONN => {}
        Err(e) => log::warn!("Failed to disconnect: {:?}", e),
    }
}

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
struct Link {
    deadlines: std::sync::Mutex<Deadlines>,
    changed: Condvar,
}

impl Link {
    fn new() -> Self {
        Self { deadlines: std::sync::Mutex::new(Deadlines::default()), changed: Condvar::new() }
    }

    fn deadlines(&self) -> MutexGuard<'_, Deadlines> {
        self.deadlines.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Connection `handle` has been made: not accepted, dropped after [`ACCEPT_TIMEOUT`] unless it is.
    /// An `encrypted` one has been checked before (see `on_connect`): accepted or being dropped already.
    fn connected(&self, handle: u16, accepted: &AtomicBool, encrypted: bool) {
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
    fn accepted(&self, accepted: &AtomicBool) {
        let mut deadlines = self.deadlines();
        accepted.store(true, Ordering::Release);
        deadlines.accept = None;
        self.changed.notify_one();
    }

    fn disconnected(&self, accepted: &AtomicBool) {
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
    fn request_conn_params(&self, handle: u16, params: ConnParams) {
        let mut deadlines = self.deadlines();
        deadlines.conn_params = Some((handle, params));
        self.changed.notify_one();
    }

    /// The main loop went idle or active, see [`BleMouseKeyboard::set_idle`].
    fn set_idle(&self, idle: bool) {
        self.deadlines().idle = idle;
    }

    /// The host of connection `handle` has been accepted: asks it for the params of the main loop's state,
    /// the loop doesn't send them to a host accepted while it's idle.
    fn request_host_conn_params(&self, handle: u16) {
        let mut deadlines = self.deadlines();
        let params = if deadlines.idle { ConnParams::IDLE } else { ConnParams::ACTIVE };
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

    fn advertising_started(&self) {
        let mut deadlines = self.deadlines();
        deadlines.advertise = None;
        deadlines.advertise_backoff = Duration::ZERO;
        self.changed.notify_one();
    }

    /// Starting advertising failed: it's tried again after a wait, longer on every failure.
    fn retry_advertising(&self) {
        let mut deadlines = self.deadlines();
        let backoff = (deadlines.advertise_backoff * 2).clamp(ADVERTISING_RETRY_MIN, ADVERTISING_RETRY_MAX);
        deadlines.advertise_backoff = backoff;
        deadlines.advertise = Some(Instant::now() + backoff);
        self.changed.notify_one();
    }

    fn stop(&self) {
        self.deadlines().stop = true;
        self.changed.notify_one();
    }

    fn run(&self, profiles: &std::sync::Mutex<Profiles>, accepted: &AtomicBool) {
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

/// Own address of connection `handle`, `None` if there's no such connection.
fn local_address(handle: u16) -> Option<esp_idf_sys::ble_addr_t> {
    let mut desc = esp_idf_sys::ble_gap_conn_desc::default();
    let rc = unsafe { esp_idf_sys::ble_gap_conn_find(handle, &mut desc) };
    (rc == 0).then_some(desc.our_ota_addr)
}

/// Profiles stay consistent if a holder panicked: every change is saved at once.
fn lock(profiles: &std::sync::Mutex<Profiles>) -> MutexGuard<'_, Profiles> {
    profiles.lock().unwrap_or_else(|e| e.into_inner())
}

/// Bonded hosts (Windows in particular) cache the GATT services and won't see the config service
/// added by a firmware update. Once per [`GATT_DB_VERSION`], marks Service Changed (whole handle
/// range) for every bonded host that subscribed to it: NimBLE indicates it to connected hosts and
/// persists the flag for the others, it is sent when they reconnect.
///
/// NimBLE keeps the handle range in RAM only, so the indication is armed again on every boot until
/// a bonded host has connected ([`gatt_change_delivered`]); hosts that reconnect only after a later
/// reboot may get an empty range and keep the cache, they have to be paired again.
fn signal_gatt_changed() {
    if std::fs::read_to_string(GATT_DB_FILE).is_ok_and(|v| v == GATT_DB_VERSION) {
        return;
    }
    info!("GATT database changed, indicating Service Changed to bonded hosts");
    unsafe {
        esp_idf_sys::ble_svc_gatt_changed(0x0001, 0xFFFF);
    }
}

/// A bonded host has reconnected after [`signal_gatt_changed`], it got the indication.
fn gatt_change_delivered() {
    if std::fs::read_to_string(GATT_DB_FILE).is_ok_and(|v| v == GATT_DB_VERSION) {
        return;
    }
    if let Err(e) = std::fs::write(GATT_DB_FILE, GATT_DB_VERSION) {
        log::error!("Failed to write {}: {}", GATT_DB_FILE, e);
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