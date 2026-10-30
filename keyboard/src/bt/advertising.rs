//! Advertising on the address and with the name of the active BLE profile, see [`crate::bt::profiles`].

use esp32_nimble::utilities::BleUuid;
use esp32_nimble::{BLEAdvertisementData, BLEAdvertising, BLEDevice};
use anyhow::bail;

use crate::bt::link::Link;
use crate::bt::profiles::{lock, Profiles};

/// Shown by the hosts with the profile number, see [`set_profile_name`].
const DEVICE_NAME: &str = "Nirklav Keyboard";

/// Human Interface Device service.
const HID_SERVICE_UUID: u16 = 0x1812;

/// Advertises on the address of the active profile, unless connected. If starting fails, [`Link`]
/// tries again later.
pub(super) fn advertise(profiles: &std::sync::Mutex<Profiles>, link: &Link) {
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
