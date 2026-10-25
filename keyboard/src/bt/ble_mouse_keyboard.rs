use std::sync::Arc;

use esp32_nimble::{BLEAdvertisementData, BLEHIDDevice, hid::*};
use esp32_nimble::{utilities::mutex::Mutex, BLECharacteristic, BLEServer};
use esp_idf_svc::hal::task::notification::Notifier;
use zerocopy::IntoBytes;

use crate::bt::config_service::ConfigService;
use crate::bt::conn_params::ConnParams;

use super::*;

static IRK_FILE: &str = "/spiflash/irk_keym.txt";

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
    /// `wake` wakes the main loop up on config requests.
    pub fn new(width: u8, height: u8, current_config: Vec<u8>, wake: Arc<Notifier>) -> anyhow::Result<Self> {
        unsafe {
            esp_idf_sys::ble_hs_cfg.store_gen_key_cb = Some(Self::gen_key_callback);
        }

        BLEDevice::init();
    
        let device = BLEDevice::take();
    
        if let Err(e) = BLEDevice::set_device_name("Nirklav Keyboard") {
            bail!("error on set device name [code: {:?}]", e);
        }
    
        device.set_own_addr_type(OwnAddrType::RpaPublicDefault);
        device
            .security()
            .set_auth(AuthReq::all())
            .set_passkey(32549)
            .set_io_cap(SecurityIOCap::NoInputNoOutput)
            .resolve_rpa();

        if let Err(e) = device.set_preferred_mtu(PREFERRED_MTU) {
            log::warn!("Failed to set preferred MTU: {:?}", e);
        }

        let server = device.get_server();
        server.advertise_on_disconnect(true);
        server.on_connect(|server, desc| { 
            info!("Client connected: {:?}", desc);
            
            if let Err(e) = ConnParams::ACTIVE.set(server, desc.conn_handle()) {
                log::warn!("Failed to set connection params: {:?}", e);
            }

            let mut ble_advertising = device.get_advertising().lock();
            ble_advertising.stop();
        });
        server.on_authentication_complete(|_server, desc, result| {
            info!("Authentication complete: {:?}", result);
            if result.is_ok() && desc.bonded() {
                gatt_change_delivered();
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
        server.on_disconnect(move |_desc, result| {
            info!("Client disconnected: {:?}", result);
            disconnect_service.on_disconnect();
        });

        let mut ble_advertising_data = BLEAdvertisementData::new();
        ble_advertising_data
            .name("Nirklav Keyboard")
            .appearance(0x03C1)
            .add_service_uuid(hid.hid_service().lock().uuid());

        let mut ble_advertising = device.get_advertising().lock();
        ble_advertising.scan_response(false);
        
        if let Err(e) = ble_advertising.set_data(&mut ble_advertising_data) {
            bail!("error on data advertising set [code: {:?}]", e);
        }
    
        if let Err(e) = ble_advertising.start() {
            bail!("error on advertising start [code: {:?}]", e);
        }
        // The GATT server is started with the advertising.
        signal_gatt_changed();

        Ok(Self {
            server,
            input_keyboard,
            output_keyboard,
            input_media_keys,
            input_mouse,
            config_service,
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

    pub fn set_idle(&mut self) -> anyhow::Result<()> {
        self.set_conn_params(ConnParams::IDLE)
    }

    pub fn set_active(&mut self) -> anyhow::Result<()>  {
        self.set_conn_params(ConnParams::ACTIVE)
    }

    fn set_conn_params(&mut self, params: ConnParams) -> anyhow::Result<()> {
        let handle = if let Some(desc) = self.server.connections().next() {
            desc.conn_handle()
        } else {
            return Ok(())
        };

        if let Err(e) = params.set(&mut self.server, handle) {
            log::warn!("Failed to set connection params: {:?}", e);
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
        if self.connected() {
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

    pub fn unpair(&mut self) {
        info!("Clearing BLE bonds and IRK...");
        unsafe {
            esp_idf_sys::ble_store_clear();
        }
        let _ = std::fs::remove_file(IRK_FILE);
        info!("Restarting...");
        unsafe {
            esp_idf_sys::esp_restart();
        }
    }
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
        let _r = BLEDevice::deinit_full();
    }
}