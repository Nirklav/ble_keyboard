use log::error;
use protocol::config::{Combo, ComboAction, ComboKey, ComboKeyIdent, Config, KeyAction, CONFIG_VERSION};

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

/// Layout with the `Fn` key position computed at load.
pub struct Layout {
    name: String,
    rows: Vec<Vec<KeyAction>>,
    fn_coord: Option<(usize, usize)>
}

impl Layout {
    fn new(layout: &protocol::config::Layout) -> Self {
        Self {
            name: layout.name.clone(),
            rows: layout.rows.clone(),
            fn_coord: layout.fn_coord()
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get(&self, x: usize, y: usize) -> KeyAction {
        self.rows.get(y)
            .and_then(|row| row.get(x))
            .copied()
            .unwrap_or(KeyAction::None)
    }

    pub fn fn_action(&self) -> Option<KeyAction> {
        self.fn_coord.map(|(x, y)| self.get(x, y))
    }

    pub fn fn_coord(&self) -> Option<(usize, usize)> {
        self.fn_coord
    }
}

/// Runtime layouts, built from a [`Config`].
pub struct Keymap {
    layouts: Vec<Layout>
}

impl Keymap {
    pub fn from_config(config: &Config) -> Self {
        Self {
            layouts: config.layouts.iter().map(Layout::new).collect()
        }
    }

    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    pub fn get(&self, idx: usize) -> &Layout {
        if idx >= self.layouts.len() {
            error!("idx out of bounds, returning first layout.");
            return &self.layouts[0];
        }

        &self.layouts[idx]
    }
}

/// Adds the held key to the reports.
pub fn press(action: &KeyAction, keyboard: &mut BleMouseKeyboard) {
    match action {
        KeyAction::Key(key) => keyboard.press_key(*key),
        KeyAction::KeyWithMod(key, key_mod) => {
            keyboard.press_key_mod(*key_mod);
            keyboard.press_key(*key);
        },
        KeyAction::Mod(key_mod) => keyboard.press_key_mod(*key_mod),
        KeyAction::Media(media_key) => keyboard.press_media_key(*media_key),
        KeyAction::MouseKey(mouse_key) => keyboard.press_mouse(*mouse_key),
        KeyAction::MouseTravel { dx, dy } => keyboard.travel_mouse_cursor(*dx, *dy),
        KeyAction::MouseScroll { dy } => keyboard.travel_mouse_wheel(*dy),
        KeyAction::Fn { .. } | KeyAction::None => { }
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
                ]
            },
            protocol::config::Layout {
                name: "FUNCTION".to_string(),
                rows: vec![
                    vec![k!(Grave),       k!(F1),          k!(F2),          k!(F3),      k!(F4),     k!(F5),    k!(LeftBracket), k!(RightBracket), km!(Num9, LeftShift), km!(Num0, LeftShift)],
                    vec![KeyAction::None, k!(F6),          k!(F7),          k!(F8),      k!(F9),     k!(F10),   k!(Num7),        k!(Num8),         k!(Num9),             k!(KpAsterisk)],
                    vec![k!(CapsLock),    k!(F11),         k!(F12),         k!(Up),      k!(Up),     k!(Up),    k!(Num4),        k!(Num5),         k!(Num6),             k!(KpPlus)],
                    vec![KeyAction::None, KeyAction::None, KeyAction::None, k!(Left),    k!(Down),   k!(Right), k!(Num1),        k!(Num2),         k!(Num3),             k!(KpDot)],
                    vec![m!(LeftCtrl),    m!(LeftShift),   KeyAction::None, m!(LeftAlt), k!(Delete), k!(Space), k!(Num0),        k!(Minus),        k!(Slash),            k!(KpEnter)],
                ]
            },
            protocol::config::Layout {
                name: "GAME".to_string(),
                rows: vec![
                    vec![k!(Escape),   k!(Num1),      k!(Num2),      k!(Num3),    k!(Num4),      k!(Num5),  k!(Num6),  k!(RightBracket),                          k!(Minus),                                k!(Equal)],
                    vec![k!(Q),        k!(W),         k!(E),         k!(R),       k!(T),         k!(Y),     k!(U),     k!(I),                                     k!(O),                                    KeyAction::MouseScroll { dy: 10 }],
                    vec![k!(A),        k!(S),         k!(D),         k!(F),       k!(G),         k!(H),     k!(J),     k!(K),                                     k!(L),                                    KeyAction::MouseScroll { dy: -10 }],
                    vec![k!(Z),        k!(X),         k!(C),         k!(V),       k!(B),         k!(N),     k!(M),     KeyAction::MouseKey(MouseKey::Left),       KeyAction::MouseTravel { dx: 0, dy:-10 }, KeyAction::MouseKey(MouseKey::Right)],
                    vec![m!(LeftCtrl), m!(LeftShift), f!(1, 0),      m!(LeftAlt), k!(Backspace), k!(Space), k!(Enter), KeyAction::MouseTravel { dx: -10, dy: 0 }, KeyAction::MouseTravel { dx: 0, dy: 10 }, KeyAction::MouseTravel { dx: 10, dy: 0 }],
                ]
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
        ]
    }
}
