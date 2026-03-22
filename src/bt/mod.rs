use std::{ffi::c_void, fmt::Display};

use anyhow::bail;
use bitflags::bitflags;
use esp32_nimble::{enums::{AuthReq, OwnAddrType, SecurityIOCap}, BLEDevice};
use log::info;
use zerocopy_derive::{Immutable, IntoBytes};

pub mod ble_mouse_keyboard;

#[allow(unused)]
#[repr(packed)]
struct MouseReport {
    buttons: MouseKey,
    x: i8,
    y: i8,
    wheel: i8,
    h_wheel: i8
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MouseKey: u8 {
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
    first: u8,
    second: u8
}

#[derive(Clone, Copy)]
pub struct MediaKey {
    name: &'static str,
    report: MediaKeyReport
}

#[allow(unused)]
impl MediaKey {
    pub fn next_track() -> Self {
        MediaKey::create("Next track", 1, 0)
    }
    
    pub fn previous_track() -> Self {
        MediaKey::create("Previous track", 2, 0)
    }

    pub fn stop() -> Self {
        MediaKey::create("Stop", 4, 0)
    }

    pub fn play_pause() -> Self {
        MediaKey::create("Play/Pause", 8, 0)
    }

    pub fn mute() -> Self {
        MediaKey::create("Mute", 16, 0)
    }

    pub fn volume_up() -> Self {
        MediaKey::create("Volume Up", 32, 0)
    }

    pub fn volume_down() -> Self {
        MediaKey::create("Volume Down", 64, 0)
    }

    pub fn www_home() -> Self {
        MediaKey::create("WWW Home", 128, 0)
    }

    pub fn local_machine_browser() -> Self {
        MediaKey::create("Browser", 0, 1) // Opens "My Computer" on Windows
    }

    pub fn calculator() -> Self {
        MediaKey::create("Calculator", 0, 2)
    }

    pub fn www_bookmarks() -> Self {
        MediaKey::create("WWW Bookmarks", 0, 4)
    }

    pub fn www_search() -> Self {
        MediaKey::create("WWW Search", 0, 8)
    }

    pub fn www_stop() -> Self {
        MediaKey::create("WWW Stop", 0, 16)
    }

    pub fn www_back() -> Self {
        MediaKey::create("WWW Back", 0, 32)
    }

    pub fn consumer_control_configuration() -> Self {
        MediaKey::create("Consumer control config", 0, 64) // Media Selection
    }

    pub fn email_reader() -> Self {
        MediaKey::create("Email", 0, 128)
    }

    fn create(name: &'static str, first: u8, second: u8) -> Self {
        MediaKey {
            name,
            report: MediaKeyReport { first, second }
        }
    }
}

impl Display for MediaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[allow(unused)]
pub const KEYBOARD_MOD_LEFT_CTRL : u8 = 0x01;
#[allow(unused)]
pub const KEYBOARD_MOD_LEFT_SHIFT : u8 = 0x02;
#[allow(unused)]
pub const KEYBOARD_MOD_LEFT_ALT : u8 = 0x04;
#[allow(unused)]
pub const KEYBOARD_MOD_LEFT_GUI : u8 = 0x08;
#[allow(unused)]
pub const KEYBOARD_MOD_RIGHT_CTRL : u8 = 0x10;
#[allow(unused)]
pub const KEYBOARD_MOD_RIGHT_SHIFT : u8 = 0x20;
#[allow(unused)]
pub const KEYBOARD_MOD_RIGHT_ALT : u8 = 0x40;
#[allow(unused)]
pub const KEYBOARD_MOD_RIGHT_GUI : u8 = 0x80;

const KEYBOARD_KEY_SHIFT: u8 = 0x80;
const KEYBOARD_ASCII_MAP: &[u8] = &[
    0x00,                  // NUL
    0x00,                  // SOH
    0x00,                  // STX
    0x00,                  // ETX
    0x00,                  // EOT
    0x00,                  // ENQ
    0x00,                  // ACK
    0x00,                  // BEL
    0x2a,                  // BS	Backspace
    0x2b,                  // TAB	Tab
    0x28,                  // LF	Enter
    0x00,                  // VT
    0x00,                  // FF
    0x00,                  // CR
    0x00,                  // SO
    0x00,                  // SI
    0x00,                  // DEL
    0x00,                  // DC1
    0x00,                  // DC2
    0x00,                  // DC3
    0x00,                  // DC4
    0x00,                  // NAK
    0x00,                  // SYN
    0x00,                  // ETB
    0x00,                  // CAN
    0x00,                  // EM
    0x00,                  // SUB
    0x00,                  // ESC
    0x00,                  // FS
    0x00,                  // GS
    0x00,                  // RS
    0x00,                  // US
    0x2c,                  //  ' '
    0x1e | KEYBOARD_KEY_SHIFT, // !
    0x34 | KEYBOARD_KEY_SHIFT, // "
    0x20 | KEYBOARD_KEY_SHIFT, // #
    0x21 | KEYBOARD_KEY_SHIFT, // $
    0x22 | KEYBOARD_KEY_SHIFT, // %
    0x24 | KEYBOARD_KEY_SHIFT, // &
    0x34,                  // '
    0x26 | KEYBOARD_KEY_SHIFT, // (
    0x27 | KEYBOARD_KEY_SHIFT, // )
    0x25 | KEYBOARD_KEY_SHIFT, // *
    0x2e | KEYBOARD_KEY_SHIFT, // +
    0x36,                  // ,
    0x2d,                  // -
    0x37,                  // .
    0x38,                  // /
    0x27,                  // 0
    0x1e,                  // 1
    0x1f,                  // 2
    0x20,                  // 3
    0x21,                  // 4
    0x22,                  // 5
    0x23,                  // 6
    0x24,                  // 7
    0x25,                  // 8
    0x26,                  // 9
    0x33 | KEYBOARD_KEY_SHIFT, // :
    0x33,                  // ;
    0x36 | KEYBOARD_KEY_SHIFT, // <
    0x2e,                  // =
    0x37 | KEYBOARD_KEY_SHIFT, // >
    0x38 | KEYBOARD_KEY_SHIFT, // ?
    0x1f | KEYBOARD_KEY_SHIFT, // @
    0x04 | KEYBOARD_KEY_SHIFT, // A
    0x05 | KEYBOARD_KEY_SHIFT, // B
    0x06 | KEYBOARD_KEY_SHIFT, // C
    0x07 | KEYBOARD_KEY_SHIFT, // D
    0x08 | KEYBOARD_KEY_SHIFT, // E
    0x09 | KEYBOARD_KEY_SHIFT, // F
    0x0a | KEYBOARD_KEY_SHIFT, // G
    0x0b | KEYBOARD_KEY_SHIFT, // H
    0x0c | KEYBOARD_KEY_SHIFT, // I
    0x0d | KEYBOARD_KEY_SHIFT, // J
    0x0e | KEYBOARD_KEY_SHIFT, // K
    0x0f | KEYBOARD_KEY_SHIFT, // L
    0x10 | KEYBOARD_KEY_SHIFT, // M
    0x11 | KEYBOARD_KEY_SHIFT, // N
    0x12 | KEYBOARD_KEY_SHIFT, // O
    0x13 | KEYBOARD_KEY_SHIFT, // P
    0x14 | KEYBOARD_KEY_SHIFT, // Q
    0x15 | KEYBOARD_KEY_SHIFT, // R
    0x16 | KEYBOARD_KEY_SHIFT, // S
    0x17 | KEYBOARD_KEY_SHIFT, // T
    0x18 | KEYBOARD_KEY_SHIFT, // U
    0x19 | KEYBOARD_KEY_SHIFT, // V
    0x1a | KEYBOARD_KEY_SHIFT, // W
    0x1b | KEYBOARD_KEY_SHIFT, // X
    0x1c | KEYBOARD_KEY_SHIFT, // Y
    0x1d | KEYBOARD_KEY_SHIFT, // Z
    0x2f,                  // [
    0x31,                  // bslash
    0x30,                  // ]
    0x23 | KEYBOARD_KEY_SHIFT, // ^
    0x2d | KEYBOARD_KEY_SHIFT, // _
    0x35,                  // `
    0x04,                  // a
    0x05,                  // b
    0x06,                  // c
    0x07,                  // d
    0x08,                  // e
    0x09,                  // f
    0x0a,                  // g
    0x0b,                  // h
    0x0c,                  // i
    0x0d,                  // j
    0x0e,                  // k
    0x0f,                  // l
    0x10,                  // m
    0x11,                  // n
    0x12,                  // o
    0x13,                  // p
    0x14,                  // q
    0x15,                  // r
    0x16,                  // s
    0x17,                  // t
    0x18,                  // u
    0x19,                  // v
    0x1a,                  // w
    0x1b,                  // x
    0x1c,                  // y
    0x1d,                  // z
    0x2f | KEYBOARD_KEY_SHIFT, // {
    0x31 | KEYBOARD_KEY_SHIFT, // |
    0x30 | KEYBOARD_KEY_SHIFT, // }
    0x35 | KEYBOARD_KEY_SHIFT, // ~
    0,                     // DEL
];