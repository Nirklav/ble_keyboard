//! Advertising on the address and with the name of the active BLE profile, see [`crate::bt::profiles`].

use std::ffi::{c_int, c_void};

use esp32_nimble::utilities::BleUuid;
use esp32_nimble::{BLEAdvertisementData, BLEAdvertising, BLEDevice};
use anyhow::bail;

use crate::bt::link::Link;

/// Shown by the hosts with the profile number, see [`set_profile_name`].
const DEVICE_NAME: &str = "Nirklav Keyboard";

/// Human Interface Device service.
const HID_SERVICE_UUID: u16 = 0x1812;

// These ESP-IDF NimBLE functions are not exposed by the Rust bindings. The iterator requires
// the host mutex, and can report the peripheral link before the GAP CONNECT callback does.
unsafe extern "C" {
    fn ble_hs_lock();
    fn ble_hs_unlock();
    fn ble_gap_conn_foreach_handle(callback: esp_idf_sys::ble_gap_conn_foreach_handle_fn, arg: *mut c_void);
}

unsafe extern "C" fn mark_connection(_handle: u16, arg: *mut c_void) -> c_int {
    *arg.cast::<bool>() = true;
    1
}

fn connecting() -> bool {
    let mut connected = false;
    unsafe {
        ble_hs_lock();
        ble_gap_conn_foreach_handle(Some(mark_connection), (&mut connected as *mut bool).cast());
        ble_hs_unlock();
    }
    connected
}

/// Advertises on the address of the active profile, unless connected. If starting fails, [`Link`]
/// tries again later.
pub(super) fn advertise(link: &Link) {
    let mut advertising = BLEDevice::take().get_advertising().lock();
    // CONNECT takes this lock before it publishes the host.
    if link.has_host() {
        return;
    }
    if restart_advertising(&mut advertising, link) {
        link.advertising_started();
    } else {
        link.retry_advertising();
    }
}

/// Advertises on the address of the active profile unless connected, a running advertising is restarted.
/// `false` if starting it failed.
fn restart_advertising(advertising: &mut BLEAdvertising, link: &Link) -> bool {
    if advertising.is_advertising() {
        if let Err(e) = advertising.stop() {
            // Retry the restart, including the case where a connection stopped advertising.
            if e.code() != esp_idf_sys::BLE_HS_EALREADY {
                log::warn!("Failed to stop advertising: {:?}", e);
            }
            return false;
        }
    }
    // A link can already exist before CONNECT validates it. Legacy advertising connections
    // derive their own address from NimBLE's configured address, so preserve it while connected.
    if connecting() {
        return false;
    }
    let index = link.apply_profile_address();
    if let Err(e) = set_profile_name(advertising, index) {
        log::warn!("{:?}", e);
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
pub(super) fn set_profile_name(advertising: &mut BLEAdvertising, index: u8) -> anyhow::Result<()> {
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
