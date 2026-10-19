//! Keys the keyboard can report to the host.
//!
//! Every type serializes with its variant / flag names in human-readable formats
//! (TOML: `"A"`, `"LeftCtrl | LeftShift"`) and compactly in binary formats
//! (postcard: variant index or the raw bits).
//!
//! Binary compatibility: new [`Key`] and [`MediaKey`] variants are only appended,
//! the variant index is what goes over the wire.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Mouse buttons, bits of the mouse report `buttons` byte.
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

bitflags! {
    /// Keyboard modifiers, bits of the keyboard report `modifiers` byte.
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

impl Serialize for MouseKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        bitflags::serde::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for MouseKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        bitflags::serde::deserialize(deserializer)
    }
}

impl Serialize for KeyMod {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        bitflags::serde::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for KeyMod {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        bitflags::serde::deserialize(deserializer)
    }
}

/// Consumer control key, one bit of the media keys report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKey {
    NextTrack,
    PreviousTrack,
    Stop,
    PlayPause,
    Mute,
    VolumeUp,
    VolumeDown,
    Calculator,
}

impl MediaKey {
    pub const ALL: [MediaKey; 8] = [
        MediaKey::NextTrack,
        MediaKey::PreviousTrack,
        MediaKey::Stop,
        MediaKey::PlayPause,
        MediaKey::Mute,
        MediaKey::VolumeUp,
        MediaKey::VolumeDown,
        MediaKey::Calculator,
    ];

    /// Bit of the key in the media keys report, matches the order of usages in the HID descriptor.
    pub const fn report_bit(self) -> u8 {
        match self {
            MediaKey::NextTrack => 1,
            MediaKey::PreviousTrack => 2,
            MediaKey::Stop => 4,
            MediaKey::PlayPause => 8,
            MediaKey::Mute => 16,
            MediaKey::VolumeUp => 32,
            MediaKey::VolumeDown => 64,
            MediaKey::Calculator => 128,
        }
    }

    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            MediaKey::NextTrack => "Next track",
            MediaKey::PreviousTrack => "Previous track",
            MediaKey::Stop => "Stop",
            MediaKey::PlayPause => "Play/Pause",
            MediaKey::Mute => "Mute",
            MediaKey::VolumeUp => "Volume Up",
            MediaKey::VolumeDown => "Volume Down",
            MediaKey::Calculator => "Calculator",
        }
    }
}

impl core::fmt::Display for MediaKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Keyboard/keypad page usage ID (HID usage tables, page 0x07).
///
/// Variants are contiguous from 0, so the serde variant index equals the usage ID.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl Key {
    /// HID usage ID.
    pub const fn usage(self) -> u8 {
        self as u8
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_key_bits_are_distinct_and_ordered() {
        for (i, key) in MediaKey::ALL.iter().enumerate() {
            assert_eq!(key.report_bit(), 1 << i);
        }
    }

    #[test]
    fn key_postcard_index_is_usage_id() {
        for key in [Key::None, Key::A, Key::Num0, Key::F24] {
            let bytes = postcard::to_slice(&key, &mut [0u8; 4]).unwrap().to_vec();
            assert_eq!(bytes, [key as u8]);
        }
    }

    #[test]
    fn modifiers_postcard_are_bits() {
        let mods = KeyMod::LeftCtrl | KeyMod::RightGui;
        let mut buf = [0u8; 4];
        assert_eq!(postcard::to_slice(&mods, &mut buf).unwrap(), &[0x81]);
        assert_eq!(postcard::from_bytes::<KeyMod>(&[0x81]).unwrap(), mods);
    }
}
