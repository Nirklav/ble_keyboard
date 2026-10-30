//! The HID reports: the report map (keyboard, media keys, mouse and the config collection of [`protocol::hid`])
//! and the reports the main loop fills.

use esp32_nimble::hid::*;
use protocol::config::KeyAction;
use protocol::motion::Step;
use zerocopy::IntoBytes;
use zerocopy_derive::{Immutable, IntoBytes};

pub(super) const KEYBOARD_ID: u8 = 0x01;
pub(super) const MEDIA_KEYS_ID: u8 = 0x02;
pub(super) const MOUSE_ID: u8 = 0x03;
pub(super) const CONFIG_ID: u8 = protocol::hid::REPORT_ID;
pub(super) const CONFIG_REPORT_LEN: u8 = protocol::hid::REPORT_LEN as u8;

pub(super) const HID_REPORT_DESCRIPTOR: &[u8] = hid!(
    (USAGE_PAGE, 0x01),         // Generic Desktop Ctrls
    (USAGE, 0x06),              // Keyboard
    (COLLECTION, 0x01),         // Application
    // ------------------------------------------------- Keyboard
    (REPORT_ID, KEYBOARD_ID),
    (USAGE_PAGE, 0x07),         // Kbrd/Keypad
    (USAGE_MINIMUM, 0xE0),
    (USAGE_MAXIMUM, 0xE7),
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x01),
    (REPORT_SIZE, 0x01),
    (REPORT_COUNT, 0x08),
    (HIDINPUT, 0x02),           // Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position
    (REPORT_COUNT, 0x01),       // 1 byte (Reserved)
    (REPORT_SIZE, 0x08),
    (HIDINPUT, 0x01),           // Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position
    (REPORT_COUNT, 0x05),       // 5 bits (Num lock, Caps lock, Scroll lock, Compose, Kana)
    (REPORT_SIZE, 0x01),
    (USAGE_PAGE, 0x08),         // LEDs
    (USAGE_MINIMUM, 0x01),      // Num Lock
    (USAGE_MAXIMUM, 0x05),      // Kana
    (HIDOUTPUT, 0x02),          // Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position,Non-volatile
    (REPORT_COUNT, 0x01),       // 3 bits (Padding)
    (REPORT_SIZE, 0x03),
    (HIDOUTPUT, 0x01),          // Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position,Non-volatile
    (REPORT_COUNT, 0x06),       // 6 bytes (Keys)
    (REPORT_SIZE, 0x08),
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x73),    // up to F24
    (USAGE_PAGE, 0x07),         // Kbrd/Keypad
    (USAGE_MINIMUM, 0x00),
    (USAGE_MAXIMUM, 0x73),
    (HIDINPUT, 0x00),           // Data,Array,Abs,No Wrap,Linear,Preferred State,No Null Position
    (END_COLLECTION),
    // ------------------------------------------------- Media Keys
    (USAGE_PAGE, 0x0C),         // Consumer
    (USAGE, 0x01),              // Consumer Control
    (COLLECTION, 0x01),         // Application
    (REPORT_ID, MEDIA_KEYS_ID),
    (USAGE_PAGE, 0x0C),         // Consumer
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x01),
    (REPORT_SIZE, 0x01),
    (REPORT_COUNT, 0x08),
    (USAGE, 0xB5),              // Scan Next Track; bit 0
    (USAGE, 0xB6),              // Scan Previous Track; bit 1
    (USAGE, 0xB7),              // Stop; bit 2
    (USAGE, 0xCD),              // Play/Pause; bit 3
    (USAGE, 0xE2),              // Mute; bit 4
    (USAGE, 0xE9),              // Volume Increment; bit 5
    (USAGE, 0xEA),              // Volume Decrement; bit 6
    (USAGE, 0x92, 0x01),        // Calculator; bit 7
    (HIDINPUT, 0x02),           // Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position
    (END_COLLECTION),
    // ------------------------------------------------- Mouse
    (USAGE_PAGE, 0x01),         // Generic Desktop
    (USAGE, 0x02),              // Mouse
    (COLLECTION, 0x01),         // Application
    (REPORT_ID, MOUSE_ID),
    (USAGE, 0x01),              // Pointer
    (COLLECTION, 0x00),         // Physical
    // ------------------------------------------------- Buttons (Left, Right, Middle, Back, Forward)
    (USAGE_PAGE, 0x09),         // Button
    (USAGE_MINIMUM, 0x01),      // Button 1
    (USAGE_MAXIMUM, 0x05),      // Button 5
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0x01),
    (REPORT_SIZE, 0x01),
    (REPORT_COUNT, 0x05),
    (HIDINPUT, 0x02),           // Data, Variable, Absolute; 5 button bits
    // ------------------------------------------------- Padding
    (REPORT_SIZE, 0x03),
    (REPORT_COUNT, 0x01),
    (HIDINPUT, 0x03),           // Constant, Variable, Absolute; 3 bit padding
    // ------------------------------------------------- X/Y position, Wheel
    (USAGE_PAGE, 0x01),         // Generic Desktop
    (USAGE, 0x30),              // X
    (USAGE, 0x31),              // Y
    (USAGE, 0x38),              // Wheel
    (LOGICAL_MINIMUM, 0x81),    // -127
    (LOGICAL_MAXIMUM, 0x7f),
    (REPORT_SIZE, 0x08),
    (REPORT_COUNT, 0x03),
    (HIDINPUT, 0x06),           // Data, Variable, Relative; 3 bytes (X,Y,Wheel)
    // ------------------------------------------------- Horizontal wheel
    (USAGE_PAGE, 0x0c),         // Consumer Devices
    (USAGE, 0x38, 0x02),        // AC Pan
    (LOGICAL_MINIMUM, 0x81),    // -127
    (LOGICAL_MAXIMUM, 0x7f),
    (REPORT_SIZE, 0x08),
    (REPORT_COUNT, 0x01),
    (HIDINPUT, 0x06),           // Data, Var, Rel
    (END_COLLECTION),
    (END_COLLECTION),
    // ------------------------------------------------- Config, see `protocol::hid`
    (USAGE_PAGE, 0x4B, 0xFF),   // Vendor 0xFF4B
    (USAGE, 0x01),
    (COLLECTION, 0x01),         // Application
    (REPORT_ID, CONFIG_ID),
    (LOGICAL_MINIMUM, 0x00),
    (LOGICAL_MAXIMUM, 0xFF, 0x00), // 255
    (REPORT_SIZE, 0x08),
    (REPORT_COUNT, CONFIG_REPORT_LEN),
    (USAGE, 0x02),              // Response
    (HIDINPUT, 0x02),           // Data,Var,Abs
    (USAGE, 0x03),              // Request
    (HIDOUTPUT, 0x02),          // Data,Var,Abs
    (END_COLLECTION)
);

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

/// Input report sent by [`Reports::flush`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    Keyboard,
    MediaKeys,
    Mouse,
}

/// The next reports, filled while the keys are processed, and the last ones sent.
#[derive(Default)]
pub struct Reports {
    key: KeyReport,
    media_key: MediaKeyReport,
    mouse: MouseReport,

    sent_key: KeyReport,
    sent_media_key: MediaKeyReport,
    sent_mouse_buttons: u8,
}

impl Reports {
    /// Adds a held key to the reports. Mouse travel and scroll are speeds (see [`protocol::motion`]), their steps
    /// come with [`Reports::travel`]. Layer, tap-hold and BLE profile actions are resolved by
    /// [`protocol::keymap::Keymap`] and never reach the report, they do nothing here.
    pub fn press(&mut self, action: &KeyAction) {
        match action {
            KeyAction::Key(key) => self.press_key(key.usage()),
            KeyAction::KeyWithMod(key, key_mod) => {
                self.key.modifiers |= key_mod.bits();
                self.press_key(key.usage());
            },
            KeyAction::Mod(key_mod) => self.key.modifiers |= key_mod.bits(),
            KeyAction::Media(media_key) => self.media_key.keys |= media_key.report_bit(),
            KeyAction::MouseKey(mouse_key) => self.mouse.buttons |= mouse_key.bits(),
            KeyAction::MouseTravel { .. }
            | KeyAction::MouseScroll { .. }
            | KeyAction::Fn { .. }
            | KeyAction::None
            | KeyAction::Transparent
            | KeyAction::Momentary(_)
            | KeyAction::Toggle(_)
            | KeyAction::To(_)
            | KeyAction::ModTap { .. }
            | KeyAction::LayerTap { .. }
            | KeyAction::BtSelect(_)
            | KeyAction::BtNext
            | KeyAction::BtPrev
            | KeyAction::BtClear => { }
        }
    }

    fn press_key(&mut self, usage: u8) {
        if let Some(slot) = self.key.keys.iter_mut().find(|slot| **slot == 0) {
            *slot = usage;
        }
    }

    /// Moves the cursor and the wheel by `step`.
    pub fn travel(&mut self, step: Step) {
        self.mouse.x = self.mouse.x.saturating_add(step.x);
        self.mouse.y = self.mouse.y.saturating_add(step.y);
        self.mouse.wheel = self.mouse.wheel.saturating_add(step.wheel);
    }

    /// Passes to `send` the reports that differ from the ones sent last, the mouse one also while it moves.
    /// Without `send` (no host gets them) nothing is sent, so a host connecting later gets the keys that are still
    /// held. The next reports start empty.
    pub fn flush(&mut self, send: Option<impl FnMut(Report, &[u8])>) {
        if let Some(mut send) = send {
            if self.key != self.sent_key {
                send(Report::Keyboard, self.key.as_bytes());
            }

            if self.media_key != self.sent_media_key {
                send(Report::MediaKeys, self.media_key.as_bytes());
            }

            let mouse = &self.mouse;
            let moved = mouse.x != 0 || mouse.y != 0 || mouse.wheel != 0 || mouse.h_wheel != 0;
            if moved || mouse.buttons != self.sent_mouse_buttons {
                send(Report::Mouse, self.mouse.as_bytes());
            }

            self.sent_key = self.key;
            self.sent_media_key = self.media_key;
            self.sent_mouse_buttons = self.mouse.buttons;
        } else {
            self.sent_key = KeyReport::default();
            self.sent_media_key = MediaKeyReport::default();
            self.sent_mouse_buttons = 0;
        }

        self.clear();
    }

    /// Nothing pressed in the next reports.
    pub fn clear(&mut self) {
        self.key = KeyReport::default();
        self.media_key = MediaKeyReport::default();
        self.mouse = MouseReport::default();
    }
}
