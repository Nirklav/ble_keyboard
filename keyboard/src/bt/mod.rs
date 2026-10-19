use std::ffi::c_void;

use anyhow::bail;
use esp32_nimble::{enums::{AuthReq, OwnAddrType, SecurityIOCap}, BLEDevice};
use log::info;
use zerocopy_derive::{Immutable, IntoBytes};

pub use protocol::keys::{Key, KeyMod, MediaKey, MouseKey};

pub mod ble_mouse_keyboard;
pub mod config_service;
mod conn_params;

#[allow(unused)]
#[repr(packed)]
#[derive(Default, IntoBytes, Immutable)]
struct MouseReport {
    buttons: u8,
    x: i8,
    y: i8,
    wheel: i8,
    h_wheel: i8
}

#[allow(unused)]
#[repr(packed)]
#[derive(Clone, Copy, PartialEq, Default, IntoBytes, Immutable)]
struct KeyReport {
    modifiers: u8,
    reserved: u8,
    keys: [u8; 6],
}

#[allow(unused)]
#[repr(packed)]
#[derive(Clone, Copy, PartialEq, Default, IntoBytes, Immutable)]
struct MediaKeyReport {
    keys: u8
}
