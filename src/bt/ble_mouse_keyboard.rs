use std::sync::Arc;

use esp32_nimble::{BLEAdvertisementData, BLEHIDDevice, hid::*};
use esp32_nimble::{utilities::mutex::Mutex, BLECharacteristic, BLEServer};
use zerocopy::IntoBytes;

use crate::bt::conn_params::ConnParams;

use super::*;

static IRK_FILE: &str = "/spiflash/irk_keym.txt";

const KEYBOARD_ID: u8 = 0x01;
const MEDIA_KEYS_ID: u8 = 0x02;
const MOUSE_ID: u8 = 0x03;

const HID_REPORT_DESCRIPTOR: &[u8] = hid!(
    (USAGE_PAGE, 0x01),         // USAGE_PAGE (Generic Desktop Ctrls)
    (USAGE, 0x06),              // USAGE (Keyboard)
    (COLLECTION, 0x01),         // COLLECTION (Application)
    // ------------------------------------------------- Keyboard
    (REPORT_ID, KEYBOARD_ID),   //   REPORT_ID (1)
    (USAGE_PAGE, 0x07),         //   USAGE_PAGE (Kbrd/Keypad)
    (USAGE_MINIMUM, 0xE0),      //   USAGE_MINIMUM (0xE0)
    (USAGE_MAXIMUM, 0xE7),      //   USAGE_MAXIMUM (0xE7)
    (LOGICAL_MINIMUM, 0x00),    //   LOGICAL_MINIMUM (0)
    (LOGICAL_MAXIMUM, 0x01),    //   Logical Maximum (1)
    (REPORT_SIZE, 0x01),        //   REPORT_SIZE (1)
    (REPORT_COUNT, 0x08),       //   REPORT_COUNT (8)
    (HIDINPUT, 0x02),           //   INPUT (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    (REPORT_COUNT, 0x01),       //   REPORT_COUNT (1) ; 1 byte (Reserved)
    (REPORT_SIZE, 0x08),        //   REPORT_SIZE (8)
    (HIDINPUT, 0x01),           //   INPUT (Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position)
    (REPORT_COUNT, 0x05),       //   REPORT_COUNT (5) ; 5 bits (Num lock, Caps lock, Scroll lock, Compose, Kana)
    (REPORT_SIZE, 0x01),        //   REPORT_SIZE (1)
    (USAGE_PAGE, 0x08),         //   USAGE_PAGE (LEDs)
    (USAGE_MINIMUM, 0x01),      //   USAGE_MINIMUM (0x01) ; Num Lock
    (USAGE_MAXIMUM, 0x05),      //   USAGE_MAXIMUM (0x05) ; Kana
    (HIDOUTPUT, 0x02),          //   OUTPUT (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position,Non-volatile)
    (REPORT_COUNT, 0x01),       //   REPORT_COUNT (1) ; 3 bits (Padding)
    (REPORT_SIZE, 0x03),        //   REPORT_SIZE (3)
    (HIDOUTPUT, 0x01),          //   OUTPUT (Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position,Non-volatile)
    (REPORT_COUNT, 0x06),       //   REPORT_COUNT (6) ; 6 bytes (Keys)
    (REPORT_SIZE, 0x08),        //   REPORT_SIZE(8)
    (LOGICAL_MINIMUM, 0x00),    //   LOGICAL_MINIMUM(0)
    (LOGICAL_MAXIMUM, 0x65),    //   LOGICAL_MAXIMUM(0x65) ; 101 keys
    (USAGE_PAGE, 0x07),         //   USAGE_PAGE (Kbrd/Keypad)
    (USAGE_MINIMUM, 0x00),      //   USAGE_MINIMUM (0)
    (USAGE_MAXIMUM, 0x65),      //   USAGE_MAXIMUM (0x65)
    (HIDINPUT, 0x00),           //   INPUT (Data,Array,Abs,No Wrap,Linear,Preferred State,No Null Position)
    (END_COLLECTION),           // END_COLLECTION
    // ------------------------------------------------- Media Keys
    (USAGE_PAGE, 0x0C),         // USAGE_PAGE (Consumer)
    (USAGE, 0x01),              // USAGE (Consumer Control)
    (COLLECTION, 0x01),         // COLLECTION (Application)
    (REPORT_ID, MEDIA_KEYS_ID), //   REPORT_ID (2)
    (USAGE_PAGE, 0x0C),         //   USAGE_PAGE (Consumer)
    (LOGICAL_MINIMUM, 0x00),    //   LOGICAL_MINIMUM (0)
    (LOGICAL_MAXIMUM, 0x01),    //   LOGICAL_MAXIMUM (1)
    (REPORT_SIZE, 0x01),        //   REPORT_SIZE (1)
    (REPORT_COUNT, 0x08),       //   REPORT_COUNT (8)
    (USAGE, 0xB5),              //   USAGE (Scan Next Track)     ; bit 0: 1
    (USAGE, 0xB6),              //   USAGE (Scan Previous Track) ; bit 1: 2
    (USAGE, 0xB7),              //   USAGE (Stop)                ; bit 2: 4
    (USAGE, 0xCD),              //   USAGE (Play/Pause)          ; bit 3: 8
    (USAGE, 0xE2),              //   USAGE (Mute)                ; bit 4: 16
    (USAGE, 0xE9),              //   USAGE (Volume Increment)    ; bit 5: 32
    (USAGE, 0xEA),              //   USAGE (Volume Decrement)    ; bit 6: 64
    (USAGE, 0x92, 0x01),        //   Usage (Calculator)          ; bit 7: 128
    (HIDINPUT, 0x02),           // INPUT (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    (END_COLLECTION),           // END_COLLECTION
    // ------------------------------------------------- Mouse
    (USAGE_PAGE, 0x01),         // USAGE_PAGE (Generic Desktop)
    (USAGE, 0x02),              // USAGE (Mouse)
    (COLLECTION, 0x01),         // COLLECTION (Application)
    (REPORT_ID, MOUSE_ID),      //   REPORT_ID (3)
    (USAGE, 0x01),              //   USAGE (Pointer)
    (COLLECTION, 0x00),         //   COLLECTION (Physical)
    // ------------------------------------------------- Buttons (Left, Right, Middle, Back, Forward)
    (USAGE_PAGE, 0x09),         //     USAGE_PAGE (Button)
    (USAGE_MINIMUM, 0x01),      //     USAGE_MINIMUM (Button 1)
    (USAGE_MAXIMUM, 0x05),      //     USAGE_MAXIMUM (Button 5)
    (LOGICAL_MINIMUM, 0x00),    //     LOGICAL_MINIMUM (0)
    (LOGICAL_MAXIMUM, 0x01),    //     LOGICAL_MAXIMUM (1)
    (REPORT_SIZE, 0x01),        //     REPORT_SIZE (1)
    (REPORT_COUNT, 0x05),       //     REPORT_COUNT (5)
    (HIDINPUT, 0x02),           //     INPUT (Data, Variable, Absolute) ;5 button bits
    // ------------------------------------------------- Padding
    (REPORT_SIZE, 0x03),        //     REPORT_SIZE (3)
    (REPORT_COUNT, 0x01),       //     REPORT_COUNT (1)
    (HIDINPUT, 0x03),           //     INPUT (Constant, Variable, Absolute) ;3 bit padding
    // ------------------------------------------------- X/Y position, Wheel
    (USAGE_PAGE, 0x01),         //     USAGE_PAGE (Generic Desktop)
    (USAGE, 0x30),              //     USAGE (X)
    (USAGE, 0x31),              //     USAGE (Y)
    (USAGE, 0x38),              //     USAGE (Wheel)
    (LOGICAL_MINIMUM, 0x81),    //     LOGICAL_MINIMUM (-127)
    (LOGICAL_MAXIMUM, 0x7f),    //     LOGICAL_MAXIMUM (127)
    (REPORT_SIZE, 0x08),        //     REPORT_SIZE (8)
    (REPORT_COUNT, 0x03),       //     REPORT_COUNT (3)
    (HIDINPUT, 0x06),           //     INPUT (Data, Variable, Relative) ;3 bytes (X,Y,Wheel)
    // ------------------------------------------------- Horizontal wheel
    (USAGE_PAGE, 0x0c),         //     USAGE PAGE (Consumer Devices)
    (USAGE, 0x38, 0x02),        //     USAGE (AC Pan)
    (LOGICAL_MINIMUM, 0x81),    //     LOGICAL_MINIMUM (-127)
    (LOGICAL_MAXIMUM, 0x7f),    //     LOGICAL_MAXIMUM (127)
    (REPORT_SIZE, 0x08),        //     REPORT_SIZE (8)
    (REPORT_COUNT, 0x01),       //     REPORT_COUNT (1)
    (HIDINPUT, 0x06),           //     INPUT (Data, Var, Rel)
    (END_COLLECTION),           //   END_COLLECTION
    (END_COLLECTION)            // END_COLLECTION
);

#[allow(unused)]
pub struct BleMouseKeyboard {
    server: &'static mut BLEServer,
    
    input_keyboard: Arc<Mutex<BLECharacteristic>>,
    output_keyboard: Arc<Mutex<BLECharacteristic>>,
    input_media_keys: Arc<Mutex<BLECharacteristic>>,
    input_mouse: Arc<Mutex<BLECharacteristic>>,

    key_report: KeyReport,
    media_key_report: MediaKeyReport,
    mouse_report: MouseReport,
}

#[allow(unused)]
impl BleMouseKeyboard {
    pub fn new() -> anyhow::Result<Self> {
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

        let server = device.get_server();
        server.advertise_on_disconnect(true);
        server.on_connect(|server, desc| { 
            info!("Client connected: {:?}", desc);
            
            ConnParams::ACTIVE.set(server, desc.conn_handle()).unwrap();

            let mut ble_advertising = device.get_advertising().lock();
            ble_advertising.stop();
        });
        server.on_disconnect(|_desc, result| {
            info!("Client disconnected: {:?}", result);
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

        Ok(Self {
            server,
            input_keyboard,
            output_keyboard,
            input_media_keys,
            input_mouse,
            key_report: KeyReport {
                modifiers: 0,
                reserved: 0,
                keys: [0; 6],
            },
            media_key_report: MediaKeyReport {
                keys: 0
            },
            mouse_report: MouseReport {
                buttons: 0,
                x: 0,
                y: 0,
                wheel: 0,
                h_wheel: 0
            },
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
                info!("IRK read: {:?}", &buf);
            }
            None => {
                esp_idf_sys::esp_fill_random(buf.as_mut_ptr() as *mut c_void, 16);
                if let Err(e) = std::fs::write(IRK_FILE, &buf) {
                    log::error!("Failed to write IRK file: {}", e);
                    return esp_idf_sys::BLE_HS_ESTORE_FAIL as i32;
                }
                info!("IRK generated: {:?}", &buf);
            }
        }

        for i in 0..16 {
            (*key).__bindgen_anon_1.irk[i] = buf[i];
        }

        info!("IRK set: {:?}", &(*key).__bindgen_anon_1.irk);
        0
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
        let mut keys = &mut self.key_report.keys;
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

    pub fn clear_keys(&mut self) {
        self.key_report.modifiers = 0;
        self.key_report.keys.fill(0);
    }

    pub fn send_key_report(&self) {
        self.input_keyboard.lock().set_value(self.key_report.as_bytes()).notify();
    }

    pub fn press_media_key(&mut self, key: MediaKey) {
        self.media_key_report.keys = key.keys;
    }

    pub fn clear_media_key(&mut self) {
        self.media_key_report.keys = 0;
    }
  
    pub fn send_media_key_report(&self) {
        self.input_media_keys.lock().set_value(self.media_key_report.as_bytes()).notify();
    }

    pub fn press_mouse(&mut self, key: MouseKey) {
        self.mouse_report.buttons |= key.bits();
    }

    pub fn travel_mouse_cursor(&mut self, x: i8, y: i8) {
        self.mouse_report.x += x;
        self.mouse_report.y += y;
    }

    pub fn travel_mouse_wheel(&mut self, v: i8) {
        self.mouse_report.wheel += v;
    }

    pub fn clear_mouse(&mut self) {
        self.mouse_report.buttons = 0;
        self.mouse_report.x = 0;
        self.mouse_report.y = 0;
        self.mouse_report.wheel = 0;
        self.mouse_report.h_wheel = 0;
    }

    pub fn send_mouse_report(&self) {
        self.input_mouse.lock().set_value(self.mouse_report.as_bytes()).notify();
    }
}

impl Drop for BleMouseKeyboard {
    fn drop(&mut self) {
        let _r = BLEDevice::deinit_full();
    }
}