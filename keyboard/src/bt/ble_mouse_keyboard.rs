use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::bail;
use esp32_nimble::enums::{AuthReq, OwnAddrType, SecurityIOCap};
use esp32_nimble::{utilities::mutex::Mutex, BLECharacteristic, BLEConnDesc, BLEDevice, BLEError, BLEHIDDevice};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use protocol::config::KeyAction;
use protocol::keymap::BtCommand;
use protocol::motion::Step;

use crate::bt::advertising::{advertise, set_profile_name};
use crate::bt::conn_params::ConnParams;
use crate::bt::gatt_cache::{gatt_change_delivered, signal_gatt_changed};
use crate::bt::link::Link;
use crate::bt::profiles::{self, ProfileState, Profiles};
use crate::bt::reports::{
    Report, Reports, CONFIG_ID, HID_REPORT_DESCRIPTOR, KEYBOARD_ID, MEDIA_KEYS_ID, MOUSE_ID, WHEEL_RESOLUTION,
};
use crate::sleep_admission::{SleepReservation, SLEEP};
use crate::storage::IRK_FILE;

pub(crate) use crate::bt::connection::ConnectionId;

/// Stack of the [`Link`] thread: logging, NimBLE calls and the profiles.
const LINK_STACK_SIZE: usize = 6 * 1024;

/// ATT MTU asked for, a 247 bytes MTU fills one data length extended LL packet.
const PREFERRED_MTU: u16 = 247;

pub struct BleMouseKeyboard {
    input_keyboard: Arc<Mutex<BLECharacteristic>>,
    input_media_keys: Arc<Mutex<BLECharacteristic>>,
    input_mouse: Arc<Mutex<BLECharacteristic>>,
    /// The resolution multiplier the host sets, see [`Self::wheel_resolution`].
    feature_mouse: Arc<Mutex<BLECharacteristic>>,
    /// Wheel steps per notch of the connected host: [`WHEEL_RESOLUTION`] once it set the multiplier, else 1.
    wheel_resolution: Arc<AtomicU8>,

    /// The config collection, see [`Self::config_reports`].
    input_config: Arc<Mutex<BLECharacteristic>>,
    output_config: Arc<Mutex<BLECharacteristic>>,
    link: Arc<Link>,
    /// Runs [`Link::run`], stopped before NimBLE is.
    link_thread: Option<JoinHandle<()>>,

    reports: Reports,
}

/// The config collection of the HID device ([`protocol::hid`]): the app writes the requests to the output
/// report, the keyboard answers in the input one.
pub struct ConfigReports {
    pub output: Arc<Mutex<BLECharacteristic>>,
    pub(crate) replies: ConfigReplies,
}

#[derive(Clone)]
pub(crate) struct ConfigReplies {
    input: Arc<Mutex<BLECharacteristic>>,
    link: Arc<Link>,
}

impl ConfigReplies {
    /// Capture the origin inside the GATT write callback. During early encryption, ESP-IDF can
    /// deliver a write before CONNECT. The profile must accept the connection before it can send
    /// config requests.
    pub(crate) fn capture(&self, desc: &BLEConnDesc) -> Option<ConnectionId> {
        self.link.ready_connection(desc.conn_handle())
    }

    /// Submit a reply to its originating connection, or return `None` if it has become stale.
    /// Callers can retry a buffer failure later with the same identity.
    pub(crate) fn send(&self, id: ConnectionId, report: &[u8]) -> Option<Result<(), BLEError>> {
        self.link.send_to(id, |handle| {
            let mut input = self.input.lock();
            if input.subscribed_count() == 0 {
                return BLEError::convert(esp_idf_sys::BLE_HS_EINVAL);
            }
            input.set_value(report).notify_with(report, handle)
        })
    }
}

impl BleMouseKeyboard {
    /// Sets the HID device up, the BLE profiles are saved in `nvs`. Nothing connects until [`Self::start`]: the
    /// services of [`Self::config_reports`] are registered before.
    pub fn new(
        nvs: EspDefaultNvsPartition,
        on_host_changed: impl Fn() + Clone + Send + Sync + 'static,
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

        let link = Arc::new(Link::new(Profiles::load(nvs)));

        let server = device.get_server();
        // Advertising is started by the disconnect callback, on the address of the profile active then.
        server.advertise_on_disconnect(false);
        let connect_link = link.clone();
        let connect_host_changed = on_host_changed.clone();
        server.on_connect(move |_, desc| {
            let Some(sleep) = SLEEP.reserve() else {
                return;
            };
            info!("Client connected: {:?}", desc);
            if connect_link.connected(desc, &sleep) {
                connect_host_changed();
                gatt_change_delivered();
            }
        });
        let auth_link = link.clone();
        server.on_authentication_complete(move |_, desc, result| {
            let Some(sleep) = SLEEP.reserve() else {
                return;
            };
            info!("Authentication complete: {:?}", result);
            if result.is_ok() && desc.encrypted() && auth_link.authenticated(desc, &sleep) {
                on_host_changed();
                gatt_change_delivered();
            }
        });

        let mut hid = BLEHIDDevice::new(server);
        let input_keyboard = hid.input_report(KEYBOARD_ID);
        // The LED report, nothing reads it.
        hid.output_report(KEYBOARD_ID);
        let input_media_keys = hid.input_report(MEDIA_KEYS_ID);
        let input_mouse = hid.input_report(MOUSE_ID);
        let feature_mouse = hid.feature_report(MOUSE_ID);
        feature_mouse.lock().set_value(&[0]);
        let wheel_resolution = Arc::new(AtomicU8::new(1));
        let write_resolution = wheel_resolution.clone();
        feature_mouse.lock().on_write(move |args| {
            let Some(_sleep) = SLEEP.reserve() else {
                args.reject_with_error_code(esp_idf_sys::BLE_ATT_ERR_WRITE_NOT_PERMITTED as u8);
                return;
            };
            let multiplier = args.recv_data().first().is_some_and(|value| value & 0x03 != 0);
            info!("Wheel resolution multiplier: {}", multiplier);
            write_resolution.store(if multiplier { WHEEL_RESOLUTION } else { 1 }, Ordering::Relaxed);
        });
        let input_config = hid.input_report(CONFIG_ID);
        let output_config = hid.output_report(CONFIG_ID);

        hid.manufacturer("Nirklav");
        // Vendor ID source 0x02: a USB vendor ID.
        hid.pnp(0x02, protocol::hid::VENDOR_ID, protocol::hid::PRODUCT_ID, 0x0100);
        hid.hid_info(0x00, 0x01);

        hid.report_map(HID_REPORT_DESCRIPTOR);

        hid.set_battery_level(100);

        Ok(Self {
            input_keyboard,
            input_media_keys,
            input_mouse,
            feature_mouse,
            wheel_resolution,
            input_config,
            output_config,
            link,
            link_thread: None,
            reports: Reports::default(),
        })
    }

    /// The config collection, for the config protocol to register its handlers on before [`Self::start`].
    pub fn config_reports(&self) -> ConfigReports {
        ConfigReports {
            output: self.output_config.clone(),
            replies: ConfigReplies { input: self.input_config.clone(), link: self.link.clone() },
        }
    }

    /// Starts the server and advertising; `on_disconnect` is called after clearing the host state.
    pub(crate) fn start(
        &mut self,
        on_disconnect: impl Fn(ConnectionId, &SleepReservation<'static>) + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        let disconnect_link = self.link.clone();
        let disconnect_config_input = self.input_config.clone();
        let disconnect_feature_mouse = self.feature_mouse.clone();
        let disconnect_wheel_resolution = self.wheel_resolution.clone();
        BLEDevice::take().get_server().on_disconnect(move |desc, result| {
            let Some(sleep) = SLEEP.reserve() else {
                return;
            };
            info!("Client disconnected: {:?}", result);
            let disconnected = disconnect_link.disconnected(desc.conn_handle(), &sleep);
            // The next host sets the multiplier again if it can.
            disconnect_feature_mouse.lock().set_value(&[0]);
            disconnect_wheel_resolution.store(1, Ordering::Relaxed);
            // Clean up the config session and wake even if the host never became ready.
            if let Some(id) = disconnected {
                // The config input also supports reads. Clear the old reply before advertising again.
                disconnect_config_input.lock().set_value(&[]);
                on_disconnect(id, &sleep);
            }
            // On the address of the profile active now: the last one selected while the connection was being
            // dropped. If setting it fails, the previous address is advertised, a host connecting there is
            // disconnected and it's tried again.
            advertise(&disconnect_link);
        });

        let mut ble_advertising = BLEDevice::take().get_advertising().lock();
        ble_advertising.scan_response(false);
        set_profile_name(&mut ble_advertising, self.profile().index)?;

        if let Err(e) = ble_advertising.start() {
            bail!("error on advertising start [code: {:?}]", e);
        }
        // The GATT server is started with the advertising.
        signal_gatt_changed();

        let link = self.link.clone();
        let link_thread = std::thread::Builder::new()
            .name("ble_link".into())
            .stack_size(LINK_STACK_SIZE)
            .spawn(move || link.run())?;
        self.link_thread = Some(link_thread);
        Ok(())
    }

    unsafe extern "C" fn gen_key_callback(t: u8, key: *mut esp_idf_sys::ble_store_gen_key, _handle: u16) -> i32 {
        if t as u32 != esp_idf_sys::BLE_STORE_GEN_KEY_IRK {
            return 0;
        }
        let Some(_sleep) = SLEEP.reserve() else {
            return esp_idf_sys::BLE_HS_ESTORE_FAIL as i32;
        };

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

    /// Idle connection parameters (slave latency) for the host. Not for a connection that isn't accepted
    /// yet: it may be pairing, and a connection update would block its encryption (see `on_connect`).
    pub fn set_idle(&mut self) {
        self.link.set_idle(true);
    }

    /// Active connection parameters for the host, see [`Self::set_idle`].
    pub fn set_active(&mut self) {
        self.link.set_idle(false);
    }

    /// A firmware update is being received: the shortest interval while active ([`ConnParams::TRANSFER`]).
    pub fn set_transfer(&mut self, transfer: bool) {
        if let Some(handle) = self.link.set_transfer(transfer) {
            let interval = ConnParams::interval_ms(handle).unwrap_or(0.0);
            info!("Firmware transfer {}, connection interval {} ms", if transfer { "started" } else { "ended" }, interval);
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

    /// Wheel steps per notch the host expects, see [`WHEEL_RESOLUTION`].
    pub fn wheel_resolution(&self) -> i32 {
        self.wheel_resolution.load(Ordering::Relaxed) as i32
    }

    /// Sends the reports to an accepted host, see [`Reports::flush`].
    pub fn flush(&mut self) {
        let link = self.link.clone();
        link.with_ready_host(|handle| self.flush_to(handle));
    }

    fn flush_to(&mut self, handle: Option<u16>) {
        let input_keyboard = &self.input_keyboard;
        let input_media_keys = &self.input_media_keys;
        let input_mouse = &self.input_mouse;
        let send = handle.map(|handle| move |report: Report, bytes: &[u8]| {
            let input = match report {
                Report::Keyboard => input_keyboard,
                Report::MediaKeys => input_media_keys,
                Report::Mouse => input_mouse,
            };
            let result = input.lock().set_value(bytes).notify_with(bytes, handle);
            if let Err(e) = result {
                // EINVAL also means the host hasn't subscribed to this report.
                if e.code() != esp_idf_sys::BLE_HS_EINVAL {
                    log::warn!("Failed to notify {:?} report: {:?}", report, e);
                }
            }
        });
        self.reports.flush(send);
    }

    /// Active BLE profile.
    pub fn profile(&self) -> ProfileState {
        self.link.profile()
    }

    pub fn change_profile(&mut self, command: BtCommand) -> ProfileState {
        // The same main task runs profile commands and commits deep sleep.
        let sleep = SLEEP.reserve().expect("profile command after deep sleep commitment");
        let link = self.link.clone();
        link.change_profile(command, &sleep, |handle| {
            self.reports.clear();
            self.flush_to(handle);
        })
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
