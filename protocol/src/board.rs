//! The keyboard board, the project [`KEYBOARD_PROJECT`](crate::image::KEYBOARD_PROJECT): its key matrix and
//! what the firmware and the apps know of its keys. The factory config, the recovery combo and the boot key
//! are made for this matrix and its key caps: `Escape` at (0, 0), `LeftCtrl` at (0, 4), `Fn` at [`FN_KEY`].
//!
//! The factory config is also `desktop/examples/config.toml`, a test there keeps the two the same.

use alloc::string::ToString;
use alloc::vec;

use crate::config::{
    Combo, ComboAction, ComboKey, ComboKeyIdent, Config, JoystickButtons, KeyAction, Layout, Settings, CONFIG_VERSION,
};
use crate::keys::{Key, KeyMod, MouseKey};

/// Number of key columns.
pub const WIDTH: u8 = 10;
/// Number of key rows.
pub const HEIGHT: u8 = 5;

/// `(x, y)` of the key held at power-on to ignore the saved config: `Escape`.
pub const BOOT_KEY: (u8, u8) = (0, 0);

/// `(x, y)` of the `Fn` key of the factory layouts, the recovery combo holds it too.
pub const FN_KEY: (u8, u8) = (2, 4);

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

/// Factory reset built into the firmware, the config can't override it:
/// hold `Fn` (2, 4) and `LeftCtrl` (0, 4), press `Escape` (0, 0) 5 times.
pub fn recovery_combo() -> Combo {
    Combo {
        keys: vec![
            ComboKey { ident: ComboKeyIdent::Coord { x: FN_KEY.0, y: FN_KEY.1 }, count: 0 },
            ComboKey { ident: ComboKeyIdent::Coord { x: 0, y: 4 }, count: 0 },
            ComboKey { ident: ComboKeyIdent::Coord { x: BOOT_KEY.0, y: BOOT_KEY.1 }, count: 5 },
        ],
        action: ComboAction::ResetConfig,
    }
}

/// Factory config.
pub fn factory_config() -> Config {
    Config {
        version: CONFIG_VERSION,
        width: WIDTH,
        height: HEIGHT,
        layouts: vec![
            Layout {
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
            Layout {
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
            Layout {
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
                    ComboKey { ident: ComboKeyIdent::Coord { x: FN_KEY.0, y: FN_KEY.1 }, count: 2 },
                ],
                action: ComboAction::SwitchLayout
            },
            Combo {
                keys: vec![
                    ComboKey { ident: ComboKeyIdent::Coord { x: FN_KEY.0, y: FN_KEY.1 }, count: 0 },
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

/// Hold `Fn` ([`FN_KEY`]), press the key (`x`, `y`) twice: selects BLE profile `profile`.
fn bt_select_combo(profile: u8, x: u8, y: u8) -> Combo {
    Combo {
        keys: vec![
            ComboKey { ident: ComboKeyIdent::Coord { x: FN_KEY.0, y: FN_KEY.1 }, count: 0 },
            ComboKey { ident: ComboKeyIdent::Coord { x, y }, count: 2 },
        ],
        action: ComboAction::BtSelect(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_config_is_valid() {
        assert_eq!(factory_config().validate_for(WIDTH, HEIGHT), Ok(()));
    }

    #[test]
    fn keys_are_the_factory_ones() {
        let main = &factory_config().layouts[0];
        let at = |(x, y): (u8, u8)| main.get(x as usize, y as usize).copied();
        assert_eq!(main.fn_coord(), Some((FN_KEY.0 as usize, FN_KEY.1 as usize)));
        assert_eq!(at(BOOT_KEY), Some(KeyAction::Key(Key::Escape)));
        assert_eq!(at((0, 4)), Some(KeyAction::Mod(KeyMod::LeftCtrl)));
        let config = Config { combos: vec![recovery_combo()], ..factory_config() };
        assert_eq!(config.validate(), Ok(()));
    }
}
