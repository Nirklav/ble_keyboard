use zerocopy_derive::{Immutable, IntoBytes};

pub use protocol::keys::{Key, KeyMod, MediaKey, MouseKey};

pub mod ble_mouse_keyboard;
pub mod config_service;
pub mod profiles;
mod conn_params;

#[repr(packed)]
#[derive(Default, IntoBytes, Immutable)]
struct MouseReport {
    buttons: u8,
    x: i8,
    y: i8,
    wheel: i8,
    h_wheel: i8
}

#[repr(packed)]
#[derive(Clone, Copy, PartialEq, Default, IntoBytes, Immutable)]
struct KeyReport {
    modifiers: u8,
    reserved: u8,
    keys: [u8; 6],
}

#[repr(packed)]
#[derive(Clone, Copy, PartialEq, Default, IntoBytes, Immutable)]
struct MediaKeyReport {
    keys: u8
}
