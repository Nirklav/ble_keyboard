use protocol::config::{Combo, ComboAction, ComboKey, ComboKeyIdent, Config, JoystickButtons, KeyAction, Settings, CONFIG_VERSION};

use protocol::motion::Velocity;

use crate::bt::{Key, KeyMod, MouseKey};
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;

macro_rules! k {
    ($key:ident) => {
        KeyAction::Key(Key::$key)
    };
}

macro_rules! m {
    ($key:ident) => {
        KeyAction::Mod(KeyMod::$key)
    };
}

macro_rules! km {
    ($key:ident, $modif:ident) => {
        KeyAction::KeyWithMod(Key::$key, KeyMod::$modif)
    };
}

macro_rules! f {
    ($temporary:expr, $permanent:expr) => {
        KeyAction::Fn { temporary: $temporary, permanent: $permanent }
    };
}

/// Adds the held key to the reports. Mouse travel and scroll add their speed to `velocity` instead,
/// it's turned into the report by the time, see [`protocol::motion`]. Layer, tap-hold and BLE profile
/// actions are resolved by [`protocol::keymap::Keymap`] and never reach the report, they do nothing here.
pub fn press(action: &KeyAction, keyboard: &mut BleMouseKeyboard, velocity: &mut Velocity) {
    match action {
        KeyAction::Key(key) => keyboard.press_key(*key),
        KeyAction::KeyWithMod(key, key_mod) => {
            keyboard.press_key_mod(*key_mod);
            keyboard.press_key(*key);
        },
        KeyAction::Mod(key_mod) => keyboard.press_key_mod(*key_mod),
        KeyAction::Media(media_key) => keyboard.press_media_key(*media_key),
        KeyAction::MouseKey(mouse_key) => keyboard.press_mouse(*mouse_key),
        KeyAction::MouseTravel { .. } | KeyAction::MouseScroll { .. } => *velocity += Velocity::of(action),
        KeyAction::Fn { .. }
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

/// Factory config.
pub fn default_config() -> Config {
    Config {
        version: CONFIG_VERSION,
        width: 10,
        height: 5,
        layouts: vec![
            protocol::config::Layout {
                name: "MAIN".to_string(),
                rows: vec![
                    vec![k!(Escape),   km!(Num1, LeftShift), km!(Num2, LeftShift), km!(Num3, LeftShift), km!(Num7, LeftShift), k!(KpAsterisk), k!(LeftBracket), k!(RightBracket), k!(Minus),     k!(Equal)],
                    vec![k!(Q),        k!(W),                k!(E),                k!(R),                k!(T),                k!(Y),          k!(U),           k!(I),            k!(O),         k!(P)],
                    vec![k!(A),        k!(S),                k!(D),                k!(F),                k!(G),                k!(H),          k!(J),           k!(K),            k!(L),         k!(Semicolon)],
                    vec![k!(Z),        k!(X),                k!(C),                k!(V),                k!(B),                k!(N),          k!(M),           k!(Comma),        k!(Dot),       k!(Slash)],
                    vec![m!(LeftCtrl), m!(LeftShift),        f!(1, 2),             m!(LeftAlt),          k!(Backspace),        k!(Space),      k!(Enter),       k!(Apostrophe),   k!(Backslash), k!(Tab)],
                ],
                joystick: JoystickButtons::default(),
            },
            protocol::config::Layout {
                name: "FUNCTION".to_string(),
                rows: vec![
                    vec![k!(Grave),       k!(F1),          k!(F2),          k!(F3),      k!(F4),     k!(F5),    k!(LeftBracket), k!(RightBracket), km!(Num9, LeftShift), km!(Num0, LeftShift)],
                    vec![KeyAction::None, k!(F6),          k!(F7),          k!(F8),      k!(F9),     k!(F10),   k!(Num7),        k!(Num8),         k!(Num9),             k!(KpAsterisk)],
                    vec![k!(CapsLock),    k!(F11),         k!(F12),         k!(Up),      k!(Up),     k!(Up),    k!(Num4),        k!(Num5),         k!(Num6),             k!(KpPlus)],
                    vec![KeyAction::None, KeyAction::None, KeyAction::None, k!(Left),    k!(Down),   k!(Right), k!(Num1),        k!(Num2),         k!(Num3),             k!(KpDot)],
                    vec![m!(LeftCtrl),    m!(LeftShift),   KeyAction::None, m!(LeftAlt), k!(Delete), k!(Space), k!(Num0),        k!(Minus),        k!(Slash),            k!(KpEnter)],
                ],
                joystick: JoystickButtons::default(),
            },
            protocol::config::Layout {
                name: "GAME".to_string(),
                rows: vec![
                    vec![k!(Escape),   k!(Num1),      k!(Num2),      k!(Num3),    k!(Num4),      k!(Num5),  k!(Num6),  k!(RightBracket),                          k!(Minus),                                k!(Equal)],
                    vec![k!(Q),        k!(W),         k!(E),         k!(R),       k!(T),         k!(Y),     k!(U),     k!(I),                                     k!(O),                                    KeyAction::MouseScroll { dy: 2 }],
                    vec![k!(A),        k!(S),         k!(D),         k!(F),       k!(G),         k!(H),     k!(J),     k!(K),                                     k!(L),                                    KeyAction::MouseScroll { dy: -2 }],
                    vec![k!(Z),        k!(X),         k!(C),         k!(V),       k!(B),         k!(N),     k!(M),     KeyAction::MouseKey(MouseKey::Left),       KeyAction::MouseTravel { dx: 0, dy:-10 }, KeyAction::MouseKey(MouseKey::Right)],
                    vec![m!(LeftCtrl), m!(LeftShift), f!(1, 0),      m!(LeftAlt), k!(Backspace), k!(Space), k!(Enter), KeyAction::MouseTravel { dx: -10, dy: 0 }, KeyAction::MouseTravel { dx: 0, dy: 10 }, KeyAction::MouseTravel { dx: 10, dy: 0 }],
                ],
                joystick: JoystickButtons::default(),
            },
        ],
        combos: vec![
            Combo {
                keys: vec![
                    ComboKey { ident: ComboKeyIdent::Coord { x: 2, y: 4 }, count: 2 },
                ],
                action: ComboAction::SwitchLayout
            },
            Combo {
                keys: vec![
                    ComboKey { ident: ComboKeyIdent::Coord { x: 2, y: 4 }, count: 0 },
                    ComboKey { ident: ComboKeyIdent::Coord { x: 3, y: 3 }, count: 3 },
                ],
                action: ComboAction::Unpair
            },
            // Fn + double press of Q, Z, X, C (`None` in FUNCTION, so nothing goes to the host being left):
            // BLE profile BT 1..BT 4.
            bt_select_combo(0, 0, 1),
            bt_select_combo(1, 0, 3),
            bt_select_combo(2, 1, 3),
            bt_select_combo(3, 2, 3),
        ],
        settings: Settings::default(),
    }
}

/// Hold `Fn` (2, 4), press the key (`x`, `y`) twice: selects BLE profile `profile`.
fn bt_select_combo(profile: u8, x: u8, y: u8) -> Combo {
    Combo {
        keys: vec![
            ComboKey { ident: ComboKeyIdent::Coord { x: 2, y: 4 }, count: 0 },
            ComboKey { ident: ComboKeyIdent::Coord { x, y }, count: 2 },
        ],
        action: ComboAction::BtSelect(profile)
    }
}
