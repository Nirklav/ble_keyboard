use std::{ffi::c_void, fmt::Display};

use bitflags::bitflags;
use anyhow::bail;
use esp32_nimble::{enums::{AuthReq, OwnAddrType, SecurityIOCap}, BLEDevice};
use log::info;
use zerocopy_derive::{Immutable, IntoBytes};

pub mod ble_mouse_keyboard;
mod conn_params;

#[allow(unused)]
#[repr(packed)]
#[derive(IntoBytes, Immutable)]
struct MouseReport {
    buttons: u8,
    x: i8,
    y: i8,
    wheel: i8,
    h_wheel: i8
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MouseKey : u8 {
        const None = 0;
        const Left = 1;
        const Right = 2;
        const Middle = 4;
        const Back = 8;
        const Forward = 16;
    }
}

#[allow(unused)]
#[repr(packed)]
#[derive(IntoBytes, Immutable)]
struct KeyReport {
    modifiers: u8,
    reserved: u8,
    keys: [u8; 6],
}

#[allow(unused)]
#[repr(packed)]
#[derive(Clone, Copy, IntoBytes, Immutable)]
struct MediaKeyReport {
    keys: u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaKey {
    name: &'static str,
    keys: u8
}

#[allow(unused)]
impl MediaKey {
    pub fn next_track() -> Self {
        MediaKey::create("Next track", 1)
    }

    pub fn previous_track() -> Self {
        MediaKey::create("Previous track", 2)
    }

    pub fn stop() -> Self {
        MediaKey::create("Stop", 4)
    }

    pub fn play_pause() -> Self {
        MediaKey::create("Play/Pause", 8)
    }

    pub fn mute() -> Self {
        MediaKey::create("Mute", 16)
    }

    pub fn volume_up() -> Self {
        MediaKey::create("Volume Up", 32)
    }

    pub fn volume_down() -> Self {
        MediaKey::create("Volume Down", 64)
    }

    pub fn calculator() -> Self {
        MediaKey::create("Calculator", 128)
    }

    fn create(name: &'static str, keys: u8) -> Self {
        MediaKey {
            name,
            keys
        }
    }
}

impl Display for MediaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct KeyMod : u8 {
        const LeftCtrl = 0x01;
        const LeftShift = 0x02;
        const LeftAlt = 0x04;
        const LeftGui = 0x08;
        const RightCtrl = 0x10;
        const RightShift = 0x20;
        const RightAlt = 0x40;
        const RightGui = 0x80;
    }
}

#[allow(unused)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    None = 0x00,
    ErrorRollOver = 0x01,
    PostFail = 0x02,
    ErrorUndefined = 0x03,
    A = 0x04,
    B = 0x05,
    C = 0x06,
    D = 0x07,
    E = 0x08,
    F = 0x09,
    G = 0x0A,
    H = 0x0B,
    I = 0x0C,
    J = 0x0D,
    K = 0x0E,
    L = 0x0F,
    M = 0x10,
    N = 0x11,
    O = 0x12,
    P = 0x13,
    Q = 0x14,
    R = 0x15,
    S = 0x16,
    T = 0x17,
    U = 0x18,
    V = 0x19,
    W = 0x1A,
    X = 0x1B,
    Y = 0x1C,
    Z = 0x1D,
    Num1 = 0x1E,
    Num2 = 0x1F,
    Num3 = 0x20,
    Num4 = 0x21,
    Num5 = 0x22,
    Num6 = 0x23,
    Num7 = 0x24,
    Num8 = 0x25,
    Num9 = 0x26,
    Num0 = 0x27,
    Enter = 0x28,
    Escape = 0x29,
    Backspace = 0x2A,
    Tab = 0x2B,
    Space = 0x2C,
    Minus = 0x2D,
    Equal = 0x2E,
    LeftBracket = 0x2F,
    RightBracket = 0x30,
    Backslash = 0x31,
    NonUsHash = 0x32,
    Semicolon = 0x33,
    Apostrophe = 0x34,
    Grave = 0x35,
    Comma = 0x36,
    Dot = 0x37,
    Slash = 0x38,
    CapsLock = 0x39,
    F1 = 0x3A,
    F2 = 0x3B,
    F3 = 0x3C,
    F4 = 0x3D,
    F5 = 0x3E,
    F6 = 0x3F,
    F7 = 0x40,
    F8 = 0x41,
    F9 = 0x42,
    F10 = 0x43,
    F11 = 0x44,
    F12 = 0x45,
    PrintScreen = 0x46,
    ScrollLock = 0x47,
    Pause = 0x48,
    Insert = 0x49,
    Home = 0x4A,
    PageUp = 0x4B,
    Delete = 0x4C,
    End = 0x4D,
    PageDown = 0x4E,
    Right = 0x4F,
    Left = 0x50,
    Down = 0x51,
    Up = 0x52,
    NumLock = 0x53,
    KpSlash = 0x54,
    KpAsterisk = 0x55,
    KpMinus = 0x56,
    KpPlus = 0x57,
    KpEnter = 0x58,
    Kp1 = 0x59,
    Kp2 = 0x5A,
    Kp3 = 0x5B,
    Kp4 = 0x5C,
    Kp5 = 0x5D,
    Kp6 = 0x5E,
    Kp7 = 0x5F,
    Kp8 = 0x60,
    Kp9 = 0x61,
    Kp0 = 0x62,
    KpDot = 0x63,
    NonUsBackslash = 0x64,
    Application = 0x65,
    Power = 0x66,
    KpEqual = 0x67,
    F13 = 0x68,
    F14 = 0x69,
    F15 = 0x6A,
    F16 = 0x6B,
    F17 = 0x6C,
    F18 = 0x6D,
    F19 = 0x6E,
    F20 = 0x6F,
    F21 = 0x70,
    F22 = 0x71,
    F23 = 0x72,
    F24 = 0x73,
}