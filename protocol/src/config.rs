//! Keyboard configuration: layouts (layers) and special key combinations.
//!
//! The same [`Config`] is used everywhere:
//! - on the wire (BLE config service, see [`crate::ble`]) and in the firmware flash it is
//!   encoded with `postcard` ([`Config::to_bytes`], [`Config::from_bytes`]);
//! - in the desktop app it is a TOML file with key names.
//!
//! Compatibility: enum variants are only appended (postcard encodes the variant index),
//! any other change to the layout of these types bumps [`CONFIG_VERSION`].

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::keys::{Key, KeyMod, MediaKey, MouseKey};

pub use crate::crc::crc32;

/// Format version of [`Config`], stored in [`Config::version`].
pub const CONFIG_VERSION: u16 = 1;

/// Max number of layouts in a config.
pub const MAX_LAYOUTS: usize = 8;

/// Max number of special combinations in a config.
pub const MAX_COMBOS: usize = 16;

/// Max number of keys in one special combination.
pub const MAX_COMBO_KEYS: usize = 8;

/// Max length of a layout name in characters: the 128 px wide display with the 10x20 font fits 12.
pub const MAX_LAYOUT_NAME_LEN: usize = 12;

/// What a key does when it's pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyAction {
    Key(Key),
    KeyWithMod(Key, KeyMod),
    Mod(KeyMod),
    Media(MediaKey),
    MouseKey(MouseKey),
    /// Moves the mouse cursor every report while held.
    MouseTravel { dx: i8, dy: i8 },
    /// Scrolls the mouse wheel every report while held.
    MouseScroll { dy: i8 },
    /// Layer key: switches to layout `temporary` while held, double tap switches to `permanent`.
    /// Indices into [`Config::layouts`]. At most one per layout.
    Fn { temporary: u8, permanent: u8 },
    /// Does nothing.
    None,
}

/// One layer of the keymap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// Shown on the display, at most [`MAX_LAYOUT_NAME_LEN`] ASCII characters.
    pub name: String,
    /// [`Config::height`] rows of [`Config::width`] keys, `rows[y][x]`.
    pub rows: Vec<Vec<KeyAction>>,
}

impl Layout {
    /// Action at `(x, y)`, `None` if out of range.
    pub fn get(&self, x: usize, y: usize) -> Option<&KeyAction> {
        self.rows.get(y).and_then(|row| row.get(x))
    }

    /// Coordinate `(x, y)` of the first [`KeyAction::Fn`], scanning by columns like the firmware does.
    pub fn fn_coord(&self) -> Option<(usize, usize)> {
        let width = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        (0..width)
            .flat_map(|x| (0..self.rows.len()).map(move |y| (x, y)))
            .find(|&(x, y)| matches!(self.get(x, y), Some(KeyAction::Fn { .. })))
    }
}

/// Special key combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combo {
    /// Keys that make the combination, 1..=[`MAX_COMBO_KEYS`], at least one with `count > 0`.
    pub keys: Vec<ComboKey>,
    pub action: ComboAction,
}

/// Key of a special combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComboKey {
    pub ident: ComboKeyIdent,
    /// Required number of presses (press + release); 0 means the key must be held.
    pub count: u8,
}

/// How a combination key is matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComboKeyIdent {
    /// Physical key position.
    Coord { x: u8, y: u8 },
    /// Any key registered as this action in the current layout.
    Action(KeyAction),
}

/// What a special combination does. New variants are only appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComboAction {
    /// Switches to the next layout.
    SwitchLayout,
    /// Switches to the layout with this index.
    SetLayout(u8),
    /// Removes bonded devices.
    Unpair,
    /// Resets the config to the factory one.
    ResetConfig,
}

/// Factory reset built into the firmware, the config can't override it:
/// hold `Fn` (2, 4) and `LeftCtrl` (0, 4), press `Escape` (0, 0) 5 times.
pub fn recovery_combo() -> Combo {
    Combo {
        keys: alloc::vec![
            ComboKey { ident: ComboKeyIdent::Coord { x: 2, y: 4 }, count: 0 },
            ComboKey { ident: ComboKeyIdent::Coord { x: 0, y: 4 }, count: 0 },
            ComboKey { ident: ComboKeyIdent::Coord { x: 0, y: 0 }, count: 5 },
        ],
        action: ComboAction::ResetConfig,
    }
}

/// Full keyboard configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Must be [`CONFIG_VERSION`].
    pub version: u16,
    /// Number of key columns.
    pub width: u8,
    /// Number of key rows.
    pub height: u8,
    /// 1..=[`MAX_LAYOUTS`] layouts, the first one is active after boot.
    pub layouts: Vec<Layout>,
    /// Up to [`MAX_COMBOS`] special combinations.
    pub combos: Vec<Combo>,
}

impl Config {
    /// Encodes with postcard.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("serialization to a vec can't fail")
    }

    /// Decodes from postcard. Doesn't [`validate`](Config::validate).
    pub fn from_bytes(bytes: &[u8]) -> Result<Config, postcard::Error> {
        postcard::from_bytes(bytes)
    }

    /// Checks the version, sizes, limits and indices.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != CONFIG_VERSION {
            return Err(ValidationError::Version { expected: CONFIG_VERSION, found: self.version });
        }

        if self.width == 0 || self.height == 0 {
            return Err(ValidationError::EmptySize);
        }

        let layouts = self.layouts.len();
        if layouts == 0 {
            return Err(ValidationError::NoLayouts);
        }
        if layouts > MAX_LAYOUTS {
            return Err(ValidationError::TooManyLayouts { count: layouts, max: MAX_LAYOUTS });
        }

        for (index, layout) in self.layouts.iter().enumerate() {
            self.validate_layout(index, layout)?;
        }

        if self.combos.len() > MAX_COMBOS {
            return Err(ValidationError::TooManyCombos { count: self.combos.len(), max: MAX_COMBOS });
        }

        for (index, combo) in self.combos.iter().enumerate() {
            self.validate_combo(index, combo)?;
        }

        Ok(())
    }

    /// [`validate`](Config::validate) and check that the config is made for a `width` x `height` keyboard.
    pub fn validate_for(&self, width: u8, height: u8) -> Result<(), ValidationError> {
        self.validate()?;
        if self.width != width || self.height != height {
            return Err(ValidationError::WrongKeyboardSize {
                expected_width: width,
                expected_height: height,
                width: self.width,
                height: self.height,
            });
        }
        Ok(())
    }

    fn validate_layout(&self, layout: usize, value: &Layout) -> Result<(), ValidationError> {
        let len = value.name.chars().count();
        if len > MAX_LAYOUT_NAME_LEN {
            return Err(ValidationError::NameTooLong { layout, len, max: MAX_LAYOUT_NAME_LEN });
        }
        if !value.name.is_ascii() {
            return Err(ValidationError::NameNotAscii { layout });
        }

        if value.rows.len() != self.height as usize {
            return Err(ValidationError::Height {
                layout,
                expected: self.height as usize,
                found: value.rows.len(),
            });
        }

        let mut fn_key = None;
        for (y, row) in value.rows.iter().enumerate() {
            if row.len() != self.width as usize {
                return Err(ValidationError::Width {
                    layout,
                    y,
                    expected: self.width as usize,
                    found: row.len(),
                });
            }

            for (x, action) in row.iter().enumerate() {
                if let KeyAction::Fn { temporary, permanent } = *action {
                    if let Some((first_x, first_y)) = fn_key {
                        return Err(ValidationError::MultipleFn { layout, first: (first_x, first_y), second: (x, y) });
                    }
                    fn_key = Some((x, y));

                    for index in [temporary, permanent] {
                        if index as usize >= self.layouts.len() {
                            return Err(ValidationError::FnLayoutIndex { layout, x, y, index });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_combo(&self, combo: usize, value: &Combo) -> Result<(), ValidationError> {
        if value.keys.is_empty() {
            return Err(ValidationError::EmptyCombo { combo });
        }
        if value.keys.len() > MAX_COMBO_KEYS {
            return Err(ValidationError::TooManyComboKeys { combo, count: value.keys.len(), max: MAX_COMBO_KEYS });
        }
        if value.keys.iter().all(|key| key.count == 0) {
            return Err(ValidationError::ComboWithoutPresses { combo });
        }

        for key in &value.keys {
            if let ComboKeyIdent::Coord { x, y } = key.ident {
                if x >= self.width || y >= self.height {
                    return Err(ValidationError::ComboCoord { combo, x, y });
                }
            }
        }

        if let ComboAction::SetLayout(index) = value.action {
            if index as usize >= self.layouts.len() {
                return Err(ValidationError::ComboLayoutIndex { combo, index });
            }
        }

        Ok(())
    }
}

/// Why a [`Config`] is rejected. Indices of layouts and combos are 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    Version { expected: u16, found: u16 },
    /// `width` or `height` is 0.
    EmptySize,
    /// Config is for a different keyboard.
    WrongKeyboardSize { expected_width: u8, expected_height: u8, width: u8, height: u8 },
    NoLayouts,
    TooManyLayouts { count: usize, max: usize },
    NameTooLong { layout: usize, len: usize, max: usize },
    NameNotAscii { layout: usize },
    /// Number of rows differs from `height`.
    Height { layout: usize, expected: usize, found: usize },
    /// Row `y` length differs from `width`.
    Width { layout: usize, y: usize, expected: usize, found: usize },
    /// Layout has more than one [`KeyAction::Fn`], coordinates are `(x, y)`.
    MultipleFn { layout: usize, first: (usize, usize), second: (usize, usize) },
    /// [`KeyAction::Fn`] at `(x, y)` points to a missing layout.
    FnLayoutIndex { layout: usize, x: usize, y: usize, index: u8 },
    TooManyCombos { count: usize, max: usize },
    EmptyCombo { combo: usize },
    TooManyComboKeys { combo: usize, count: usize, max: usize },
    /// Every key must be held: the combination would fire again and again while they are.
    ComboWithoutPresses { combo: usize },
    /// [`ComboKeyIdent::Coord`] is outside the keyboard.
    ComboCoord { combo: usize, x: u8, y: u8 },
    /// [`ComboAction::SetLayout`] points to a missing layout.
    ComboLayoutIndex { combo: usize, index: u8 },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ValidationError::Version { expected, found } =>
                write!(f, "config version {found} is not supported, expected {expected}"),
            ValidationError::EmptySize =>
                write!(f, "width and height must be greater than 0"),
            ValidationError::WrongKeyboardSize { expected_width, expected_height, width, height } =>
                write!(f, "config is for a {width}x{height} keyboard, this keyboard is {expected_width}x{expected_height}"),
            ValidationError::NoLayouts =>
                write!(f, "config has no layouts"),
            ValidationError::TooManyLayouts { count, max } =>
                write!(f, "{count} layouts, max is {max}"),
            ValidationError::NameTooLong { layout, len, max } =>
                write!(f, "layout {layout}: name has {len} characters, max is {max}"),
            ValidationError::NameNotAscii { layout } =>
                write!(f, "layout {layout}: name must be ASCII"),
            ValidationError::Height { layout, expected, found } =>
                write!(f, "layout {layout}: {found} rows, expected {expected}"),
            ValidationError::Width { layout, y, expected, found } =>
                write!(f, "layout {layout}: row {y} has {found} keys, expected {expected}"),
            ValidationError::MultipleFn { layout, first, second } =>
                write!(f, "layout {layout}: more than one Fn key, at {first:?} and {second:?}"),
            ValidationError::FnLayoutIndex { layout, x, y, index } =>
                write!(f, "layout {layout}: Fn key at ({x}, {y}) points to missing layout {index}"),
            ValidationError::TooManyCombos { count, max } =>
                write!(f, "{count} combos, max is {max}"),
            ValidationError::EmptyCombo { combo } =>
                write!(f, "combo {combo}: no keys"),
            ValidationError::TooManyComboKeys { combo, count, max } =>
                write!(f, "combo {combo}: {count} keys, max is {max}"),
            ValidationError::ComboWithoutPresses { combo } =>
                write!(f, "combo {combo}: every key is held, at least one needs a number of presses"),
            ValidationError::ComboCoord { combo, x, y } =>
                write!(f, "combo {combo}: key ({x}, {y}) is outside the keyboard"),
            ValidationError::ComboLayoutIndex { combo, index } =>
                write!(f, "combo {combo}: points to missing layout {index}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ValidationError {}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::string::ToString;
    use alloc::vec;

    use super::*;

    /// 10x5 config with 3 layouts and 2 combos, close to the firmware default.
    pub(crate) fn sample() -> Config {
        let row = |action: KeyAction| vec![action; 10];
        let mut main = vec![row(KeyAction::Key(Key::A)); 5];
        main[0][0] = KeyAction::Key(Key::Escape);
        main[0][1] = KeyAction::KeyWithMod(Key::Num1, KeyMod::LeftShift);
        main[4][0] = KeyAction::Mod(KeyMod::LeftCtrl | KeyMod::LeftAlt);
        main[4][2] = KeyAction::Fn { temporary: 1, permanent: 2 };
        main[4][3] = KeyAction::None;
        main[4][4] = KeyAction::Media(MediaKey::VolumeUp);

        let mut func = vec![row(KeyAction::None); 5];
        func[0][0] = KeyAction::Key(Key::F1);

        let mut game = vec![row(KeyAction::Key(Key::W)); 5];
        game[3][7] = KeyAction::MouseKey(MouseKey::Left);
        game[3][8] = KeyAction::MouseTravel { dx: 0, dy: -10 };
        game[1][9] = KeyAction::MouseScroll { dy: 10 };
        game[4][2] = KeyAction::Fn { temporary: 1, permanent: 0 };

        Config {
            version: CONFIG_VERSION,
            width: 10,
            height: 5,
            layouts: vec![
                Layout { name: "MAIN".to_string(), rows: main },
                Layout { name: "FUNCTION".to_string(), rows: func },
                Layout { name: "GAME".to_string(), rows: game },
            ],
            combos: vec![
                Combo {
                    keys: vec![
                        ComboKey { ident: ComboKeyIdent::Coord { x: 2, y: 4 }, count: 0 },
                        ComboKey { ident: ComboKeyIdent::Coord { x: 0, y: 0 }, count: 2 },
                    ],
                    action: ComboAction::SwitchLayout,
                },
                Combo {
                    keys: vec![
                        ComboKey { ident: ComboKeyIdent::Action(KeyAction::Mod(KeyMod::LeftCtrl)), count: 0 },
                        ComboKey { ident: ComboKeyIdent::Action(KeyAction::None), count: 3 },
                    ],
                    action: ComboAction::SetLayout(2),
                },
                Combo {
                    keys: vec![ComboKey { ident: ComboKeyIdent::Coord { x: 9, y: 4 }, count: 5 }],
                    action: ComboAction::Unpair,
                },
            ],
        }
    }

    #[test]
    fn sample_is_valid() {
        assert_eq!(sample().validate(), Ok(()));
        assert_eq!(sample().validate_for(10, 5), Ok(()));
    }

    #[test]
    fn postcard_roundtrip() {
        let config = sample();
        let bytes = config.to_bytes();
        assert_eq!(Config::from_bytes(&bytes).unwrap(), config);
        assert!(Config::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn toml_roundtrip() {
        let config = sample();
        let text = toml::to_string(&config).unwrap();
        assert!(text.contains("\"LeftCtrl | LeftAlt\""), "{text}");
        assert!(text.contains("\"None\""), "{text}");
        assert!(text.contains("\"SwitchLayout\""), "{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn toml_handwritten() {
        let text = r#"
            version = 1
            width = 2
            height = 1

            [[layouts]]
            name = "MAIN"
            rows = [
                [{ Key = "A" }, { Fn = { temporary = 0, permanent = 0 } }],
            ]

            [[layouts]]
            name = "MEDIA"
            rows = [
                ["None", { Media = "PlayPause" }],
            ]

            [[combos]]
            keys = [{ ident = { Coord = { x = 1, y = 0 } }, count = 0 }, { ident = { Action = { Key = "A" } }, count = 2 }]
            action = { SetLayout = 1 }

            [[combos]]
            keys = [{ ident = { Action = "None" }, count = 1 }]
            action = "ResetConfig"
        "#;
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.validate(), Ok(()));
        assert_eq!(config.layouts[1].rows[0], [KeyAction::None, KeyAction::Media(MediaKey::PlayPause)]);
        assert_eq!(config.combos[0].action, ComboAction::SetLayout(1));
        assert_eq!(config.combos[1].action, ComboAction::ResetConfig);
    }

    #[test]
    fn fn_coord_scans_by_columns() {
        let config = sample();
        assert_eq!(config.layouts[0].fn_coord(), Some((2, 4)));
        assert_eq!(config.layouts[1].fn_coord(), None);
    }

    fn invalid(change: impl FnOnce(&mut Config)) -> ValidationError {
        let mut config = sample();
        change(&mut config);
        let err = config.validate().unwrap_err();
        assert!(!err.to_string().is_empty());
        err
    }

    #[test]
    fn validation_errors() {
        assert_eq!(invalid(|c| c.version = 99), ValidationError::Version { expected: CONFIG_VERSION, found: 99 });
        assert_eq!(invalid(|c| c.width = 0), ValidationError::EmptySize);
        assert_eq!(invalid(|c| c.layouts.clear()), ValidationError::NoLayouts);
        assert_eq!(
            invalid(|c| c.layouts = vec![c.layouts[0].clone(); 9]),
            ValidationError::TooManyLayouts { count: 9, max: MAX_LAYOUTS }
        );
        assert_eq!(
            invalid(|c| c.layouts[1].name = "ABCDEFGHIJKLM".to_string()),
            ValidationError::NameTooLong { layout: 1, len: 13, max: MAX_LAYOUT_NAME_LEN }
        );
        assert_eq!(invalid(|c| c.layouts[1].name = "Слой".to_string()), ValidationError::NameNotAscii { layout: 1 });
        assert_eq!(
            invalid(|c| { c.layouts[2].rows.pop(); }),
            ValidationError::Height { layout: 2, expected: 5, found: 4 }
        );
        assert_eq!(
            invalid(|c| { c.layouts[2].rows[3].pop(); }),
            ValidationError::Width { layout: 2, y: 3, expected: 10, found: 9 }
        );
        assert_eq!(
            invalid(|c| c.layouts[0].rows[0][5] = KeyAction::Fn { temporary: 0, permanent: 0 }),
            ValidationError::MultipleFn { layout: 0, first: (5, 0), second: (2, 4) }
        );
        assert_eq!(
            invalid(|c| c.layouts[2].rows[4][2] = KeyAction::Fn { temporary: 1, permanent: 3 }),
            ValidationError::FnLayoutIndex { layout: 2, x: 2, y: 4, index: 3 }
        );
        assert_eq!(
            invalid(|c| c.combos = vec![c.combos[0].clone(); 17]),
            ValidationError::TooManyCombos { count: 17, max: MAX_COMBOS }
        );
        assert_eq!(invalid(|c| c.combos[1].keys.clear()), ValidationError::EmptyCombo { combo: 1 });
        assert_eq!(
            invalid(|c| c.combos[1].keys = vec![c.combos[1].keys[0]; 9]),
            ValidationError::TooManyComboKeys { combo: 1, count: 9, max: MAX_COMBO_KEYS }
        );
        assert_eq!(invalid(|c| c.combos[2].keys[0].count = 0), ValidationError::ComboWithoutPresses { combo: 2 });
        assert_eq!(
            invalid(|c| c.combos[2].keys[0].ident = ComboKeyIdent::Coord { x: 10, y: 0 }),
            ValidationError::ComboCoord { combo: 2, x: 10, y: 0 }
        );
        assert_eq!(
            invalid(|c| c.combos[2].keys[0].ident = ComboKeyIdent::Coord { x: 0, y: 5 }),
            ValidationError::ComboCoord { combo: 2, x: 0, y: 5 }
        );
        assert_eq!(
            invalid(|c| c.combos[0].action = ComboAction::SetLayout(3)),
            ValidationError::ComboLayoutIndex { combo: 0, index: 3 }
        );
        assert_eq!(
            sample().validate_for(12, 5),
            Err(ValidationError::WrongKeyboardSize { expected_width: 12, expected_height: 5, width: 10, height: 5 })
        );
    }
}
