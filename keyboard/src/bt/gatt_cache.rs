//! The GATT database the bonded hosts cached: they are told once it changes.

use log::info;

use crate::storage::GATT_DB_FILE;

/// Bump on any change of the services, characteristics or the HID report map:
/// 1 - HID only, 2 - config service, 3 - keys up to F24, 4 - HID config reports, 5 - no config service.
const GATT_DB_VERSION: &str = "5";

/// Bonded hosts (Windows in particular) cache the GATT services and won't see services, characteristics
/// or a report map changed by a firmware update. Once per [`GATT_DB_VERSION`], marks Service Changed (whole handle
/// range) for every bonded host that subscribed to it: NimBLE indicates it to connected hosts and
/// persists the flag for the others, it is sent when they reconnect.
///
/// NimBLE keeps the handle range in RAM only, so the indication is armed again on every boot until
/// a bonded host has connected ([`gatt_change_delivered`]); hosts that reconnect only after a later
/// reboot may get an empty range and keep the cache, they have to be paired again.
pub(super) fn signal_gatt_changed() {
    if std::fs::read_to_string(GATT_DB_FILE).is_ok_and(|v| v == GATT_DB_VERSION) {
        return;
    }
    info!("GATT database changed, indicating Service Changed to bonded hosts");
    unsafe {
        esp_idf_sys::ble_svc_gatt_changed(0x0001, 0xFFFF);
    }
}

/// A bonded host has reconnected after [`signal_gatt_changed`], it got the indication.
pub(super) fn gatt_change_delivered() {
    if std::fs::read_to_string(GATT_DB_FILE).is_ok_and(|v| v == GATT_DB_VERSION) {
        return;
    }
    if let Err(e) = std::fs::write(GATT_DB_FILE, GATT_DB_VERSION) {
        log::error!("Failed to write {}: {}", GATT_DB_FILE, e);
    }
}
