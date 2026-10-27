//! Keyboard configuration: layouts (layers) and special key combinations.
//!
//! The same [`Config`] is used everywhere:
//! - on the wire (BLE config service, see [`crate::ble`]) and in the firmware flash it is
//!   encoded with `postcard` ([`Config::to_bytes`], [`Config::from_bytes`]);
//! - in the desktop app it is a TOML file with key names.
//!
//! Compatibility: enum variants are only appended (postcard encodes the variant index),
//! any other change to the layout of these types bumps [`CONFIG_VERSION`]. The previous layouts
//! are kept (see [`v1`], [`v2`]) and upgraded on decode, so saved configs survive a firmware update.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::keys::{Key, KeyMod, MediaKey, MouseKey};

pub use crate::crc::crc32;

/// Format version of [`Config`], stored in [`Config::version`].
pub const CONFIG_VERSION: u16 = 3;

/// Oldest [`Config::version`] [`Config::from_bytes`] still reads (and upgrades).
pub const OLDEST_CONFIG_VERSION: u16 = 1;

pub const MAX_LAYOUTS: usize = 8;

pub const MAX_COMBOS: usize = 16;

pub const MAX_COMBO_KEYS: usize = 8;

/// Max length of a layout name in characters: the 128 px wide display with the 10x20 font fits 12.
pub const MAX_LAYOUT_NAME_LEN: usize = 12;

/// Number of BLE host profiles, see [`KeyAction::BtSelect`]. Each one remembers one host;
/// the firmware keeps the bonds of all of them, `CONFIG_BT_NIMBLE_MAX_BONDS` must be larger.
pub const BT_PROFILES: usize = 4;

/// What a key does when it's pressed.
///
/// Layer actions ([`KeyAction::Fn`], [`KeyAction::Momentary`], [`KeyAction::Toggle`], [`KeyAction::To`],
/// [`KeyAction::LayerTap`]) and [`KeyAction::Transparent`] work on a base layer and a stack of layers on top
/// of it, see [`crate::keymap`].
///
/// BLE profile actions ([`KeyAction::BtSelect`], [`KeyAction::BtNext`], [`KeyAction::BtPrev`],
/// [`KeyAction::BtClear`]) switch between the hosts the keyboard remembers, see [`BT_PROFILES`].
/// A profile is bound to at most one host: selecting a bound profile reconnects to its host only,
/// an empty one waits for a new host to pair and binds it. Profiles are 0-based here and shown as
/// `BT 1`..`BT 4` on the display and in the app.
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
    /// Layer key: switches to layout `temporary` while held (like [`KeyAction::Momentary`]),
    /// double tap switches to `permanent`. Indices into [`Config::layouts`]. At most one per layout.
    Fn { temporary: u8, permanent: u8 },
    None,
    /// Does what the same key does in the next active layer below, nothing in the base layer.
    Transparent,
    /// `MO(n)`: layout `n` is on top of the active ones while held.
    Momentary(u8),
    /// `TG(n)`: a press puts layout `n` on top of the active ones, or turns it off if it's active,
    /// even if it's held by a [`KeyAction::Momentary`] (like QMK).
    Toggle(u8),
    /// `TO(n)`: a press makes layout `n` the base layer and turns every other layer off.
    To(u8),
    /// Tap sends `tap`, hold holds `mods` (at least one); [`Settings`] tell a tap from a hold.
    ModTap { mods: KeyMod, tap: Key },
    /// Tap sends `tap`, hold is [`KeyAction::Momentary`] of `layer`; [`Settings`] tell a tap from a hold.
    LayerTap { layer: u8, tap: Key },
    /// Selects BLE host profile `n` (0-based), less than [`BT_PROFILES`]: the keyboard drops the current
    /// host and connects to the one bound to the profile, or waits for a new host to pair.
    BtSelect(u8),
    /// Selects the next BLE host profile, the first one after the last.
    BtNext,
    /// Selects the previous BLE host profile, the last one before the first.
    BtPrev,
    /// Forgets the host of the current BLE profile, the profile waits for a new host to pair.
    BtClear,
}

impl KeyAction {
    /// Indices into [`Config::layouts`] the action points to.
    pub fn layers(&self) -> impl Iterator<Item = u8> {
        let (first, second) = match *self {
            KeyAction::Fn { temporary, permanent } => (Some(temporary), Some(permanent)),
            KeyAction::Momentary(layer)
            | KeyAction::Toggle(layer)
            | KeyAction::To(layer)
            | KeyAction::LayerTap { layer, .. } => (Some(layer), None),
            _ => (None, None),
        };
        first.into_iter().chain(second)
    }

    /// Applies `map` to every layout index of [`KeyAction::layers`].
    pub fn map_layers(&mut self, mut map: impl FnMut(u8) -> u8) {
        match self {
            KeyAction::Fn { temporary, permanent } => {
                *temporary = map(*temporary);
                *permanent = map(*permanent);
            }
            KeyAction::Momentary(layer)
            | KeyAction::Toggle(layer)
            | KeyAction::To(layer)
            | KeyAction::LayerTap { layer, .. } => *layer = map(*layer),
            _ => {}
        }
    }

    /// BLE profile index of [`KeyAction::BtSelect`], must be less than [`BT_PROFILES`].
    pub fn bt_profile(&self) -> Option<u8> {
        match *self {
            KeyAction::BtSelect(profile) => Some(profile),
            _ => None,
        }
    }

    /// [`KeyAction::ModTap`] with no modifiers: its hold would do nothing, yet it delays the taps.
    pub fn is_mod_tap_without_mods(&self) -> bool {
        matches!(self, KeyAction::ModTap { mods, .. } if mods.is_empty())
    }
}

/// Global key processing settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// A [`KeyAction::ModTap`] or [`KeyAction::LayerTap`] held this long is a hold.
    /// Released earlier it's a tap, unless another key was pressed and released meanwhile.
    pub tapping_term_ms: u16,
    /// A tap-hold key pressed this soon after the press of a typed key is a tap at once:
    /// no misfires while typing fast. 0 turns it off.
    pub require_prior_idle_ms: u16,
}

impl Settings {
    /// Allowed [`Settings::tapping_term_ms`].
    pub const TAPPING_TERM_MS: RangeInclusive<u16> = 50..=1000;
    /// Allowed [`Settings::require_prior_idle_ms`].
    pub const REQUIRE_PRIOR_IDLE_MS: RangeInclusive<u16> = 0..=1000;
}

impl Default for Settings {
    fn default() -> Self {
        Self { tapping_term_ms: 200, require_prior_idle_ms: 150 }
    }
}

/// One layer of the keymap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// Shown on the display, at most [`MAX_LAYOUT_NAME_LEN`] ASCII characters.
    pub name: String,
    /// [`Config::height`] rows of [`Config::width`] keys, `rows[y][x]`.
    pub rows: Vec<Vec<KeyAction>>,
    /// Joystick buttons, may be left out in TOML.
    #[serde(default)]
    pub joystick: JoystickButtons,
}

/// What the joystick buttons do in a layout. [`KeyAction::Fn`] is not allowed, the other layer actions are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoystickButtons {
    pub left: KeyAction,
    /// Pressing the stick itself.
    pub stick: KeyAction,
    pub right: KeyAction,
}

/// Button of the joystick, see [`JoystickButtons`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoystickButton {
    Left,
    Stick,
    Right,
}

impl JoystickButton {
    /// In the order they are shown, [`JoystickButton::index`] is the position here.
    pub const ALL: [JoystickButton; 3] = [JoystickButton::Left, JoystickButton::Stick, JoystickButton::Right];

    pub fn name(self) -> &'static str {
        match self {
            JoystickButton::Left => "Left",
            JoystickButton::Stick => "Stick",
            JoystickButton::Right => "Right",
        }
    }

    /// [`crate::joystick::State::buttons`] bit of the button.
    pub fn bit(self) -> u8 {
        match self {
            JoystickButton::Left => crate::joystick::BUTTON_LEFT,
            JoystickButton::Stick => crate::joystick::BUTTON_STICK,
            JoystickButton::Right => crate::joystick::BUTTON_RIGHT,
        }
    }

    /// Position in [`JoystickButton::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

impl JoystickButtons {
    pub fn get(&self, button: JoystickButton) -> &KeyAction {
        match button {
            JoystickButton::Left => &self.left,
            JoystickButton::Stick => &self.stick,
            JoystickButton::Right => &self.right,
        }
    }

    pub fn get_mut(&mut self, button: JoystickButton) -> &mut KeyAction {
        match button {
            JoystickButton::Left => &mut self.left,
            JoystickButton::Stick => &mut self.stick,
            JoystickButton::Right => &mut self.right,
        }
    }
}

impl Default for JoystickButtons {
    /// Mouse buttons: the stick is the middle one.
    fn default() -> Self {
        Self {
            left: KeyAction::MouseKey(MouseKey::Left),
            stick: KeyAction::MouseKey(MouseKey::Middle),
            right: KeyAction::MouseKey(MouseKey::Right),
        }
    }
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
    /// Any key registered as this action in the current layout. Matched against the action a key was pressed as,
    /// after the [`KeyAction::Transparent`] fall-through, so `Transparent` never matches; a [`KeyAction::ModTap`]
    /// key is matched as the `ModTap` itself, not as its tap key.
    Action(KeyAction),
}

/// What a special combination does. New variants are only appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComboAction {
    /// Switches to the next layout.
    SwitchLayout,
    /// Switches to the layout with this index.
    SetLayout(u8),
    /// Same as [`ComboAction::BtClear`], kept for older configs: forgets the host of the current
    /// BLE profile. Before the profiles it removed every bonded host.
    Unpair,
    /// Resets the config to the factory one.
    ResetConfig,
    /// Like [`KeyAction::BtSelect`]: selects BLE host profile `n`, less than [`BT_PROFILES`].
    BtSelect(u8),
    /// Like [`KeyAction::BtNext`].
    BtNext,
    /// Like [`KeyAction::BtPrev`].
    BtPrev,
    /// Like [`KeyAction::BtClear`]: forgets the host of the current BLE profile.
    BtClear,
    /// Forgets the hosts of all BLE profiles.
    BtClearAll,
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
    /// May be left out in TOML.
    #[serde(default)]
    pub settings: Settings,
}

impl Config {
    /// Encodes with postcard.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("serialization to a vec can't fail")
    }

    /// Decodes from postcard, a config of an older version is upgraded to [`CONFIG_VERSION`].
    /// Doesn't [`validate`](Config::validate).
    pub fn from_bytes(bytes: &[u8]) -> Result<Config, postcard::Error> {
        // `version` is the first field, so it can be read before the layout of the rest is known.
        let (version, _) = postcard::take_from_bytes::<u16>(bytes)?;
        match version {
            1 => postcard::from_bytes::<v1::Config>(bytes).map(Config::from),
            2 => postcard::from_bytes::<v2::Config>(bytes).map(Config::from),
            _ => postcard::from_bytes(bytes),
        }
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

        let settings = [
            ("tapping_term_ms", self.settings.tapping_term_ms, Settings::TAPPING_TERM_MS),
            ("require_prior_idle_ms", self.settings.require_prior_idle_ms, Settings::REQUIRE_PRIOR_IDLE_MS),
        ];
        for (name, value, range) in settings {
            if !range.contains(&value) {
                return Err(ValidationError::Setting { name, value, min: *range.start(), max: *range.end() });
            }
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
                let is_fn = matches!(action, KeyAction::Fn { .. });
                if is_fn {
                    if let Some((first_x, first_y)) = fn_key {
                        return Err(ValidationError::MultipleFn { layout, first: (first_x, first_y), second: (x, y) });
                    }
                    fn_key = Some((x, y));
                }

                self.check_action(action).map_err(|error| match error {
                    ActionError::LayerIndex(index) if is_fn => ValidationError::FnLayoutIndex { layout, x, y, index },
                    error => ValidationError::Key { layout, x, y, error },
                })?;
            }
        }

        for button in JoystickButton::ALL {
            let action = value.joystick.get(button);
            if matches!(action, KeyAction::Fn { .. }) {
                return Err(ValidationError::JoystickFn { layout });
            }
            self.check_action(action).map_err(|error| ValidationError::Joystick { layout, button, error })?;
        }

        Ok(())
    }

    /// Checks of a [`KeyAction`] wherever it is: a layout key, a joystick button or a combo key.
    fn check_action(&self, action: &KeyAction) -> Result<(), ActionError> {
        if let Some(index) = action.layers().find(|&index| index as usize >= self.layouts.len()) {
            return Err(ActionError::LayerIndex(index));
        }
        if let Some(index) = action.bt_profile().filter(|&index| index as usize >= BT_PROFILES) {
            return Err(ActionError::BtProfileIndex(index));
        }
        if action.is_mod_tap_without_mods() {
            return Err(ActionError::ModTapWithoutMods);
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
            match key.ident {
                ComboKeyIdent::Coord { x, y } if x >= self.width || y >= self.height => {
                    return Err(ValidationError::ComboCoord { combo, x, y });
                }
                ComboKeyIdent::Action(KeyAction::Transparent) => {
                    return Err(ValidationError::ComboTransparentKey { combo });
                }
                // Like a key of a layout, though the action is only compared: a valid config has no such keys.
                ComboKeyIdent::Action(action) => {
                    self.check_action(&action).map_err(|error| ValidationError::ComboKey { combo, error })?;
                }
                _ => {}
            }
        }

        match value.action {
            ComboAction::SetLayout(index) if index as usize >= self.layouts.len() => {
                return Err(ValidationError::ComboLayoutIndex { combo, index });
            }
            ComboAction::BtSelect(index) if index as usize >= BT_PROFILES => {
                return Err(ValidationError::ComboBtProfileIndex { combo, index });
            }
            _ => {}
        }

        Ok(())
    }
}

/// Config of version 1: layouts without the joystick buttons.
pub mod v1 {
    use alloc::string::String;
    use alloc::vec::Vec;

    use serde::{Deserialize, Serialize};

    use super::{Combo, JoystickButtons, KeyAction};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Layout {
        pub name: String,
        pub rows: Vec<Vec<KeyAction>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Config {
        pub version: u16,
        pub width: u8,
        pub height: u8,
        pub layouts: Vec<Layout>,
        pub combos: Vec<Combo>,
    }

    /// The joystick buttons are the default ones.
    impl From<Config> for super::Config {
        fn from(config: Config) -> Self {
            let layouts = config
                .layouts
                .into_iter()
                .map(|layout| super::Layout { name: layout.name, rows: layout.rows, joystick: JoystickButtons::default() })
                .collect();
            super::v2::Config { version: 2, width: config.width, height: config.height, layouts, combos: config.combos }
                .into()
        }
    }
}

/// Config of version 2: no [`Settings`].
pub mod v2 {
    use alloc::vec::Vec;

    use serde::{Deserialize, Serialize};

    use super::{Combo, Layout, Settings, CONFIG_VERSION};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Config {
        pub version: u16,
        pub width: u8,
        pub height: u8,
        pub layouts: Vec<Layout>,
        pub combos: Vec<Combo>,
    }

    /// The settings are the default ones.
    impl From<Config> for super::Config {
        fn from(config: Config) -> Self {
            Self {
                version: CONFIG_VERSION,
                width: config.width,
                height: config.height,
                layouts: config.layouts,
                combos: config.combos,
                settings: Settings::default(),
            }
        }
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
    /// [`KeyAction::Fn`] on a joystick button.
    JoystickFn { layout: usize },
    /// Action of the key at `(x, y)` is invalid; a layer index of [`KeyAction::Fn`] is [`ValidationError::FnLayoutIndex`].
    Key { layout: usize, x: usize, y: usize, error: ActionError },
    /// Action of a joystick button is invalid.
    Joystick { layout: usize, button: JoystickButton, error: ActionError },
    /// A [`Settings`] value is out of its range.
    Setting { name: &'static str, value: u16, min: u16, max: u16 },
    TooManyCombos { count: usize, max: usize },
    EmptyCombo { combo: usize },
    TooManyComboKeys { combo: usize, count: usize, max: usize },
    /// Every key must be held: the combination would fire again and again while they are.
    ComboWithoutPresses { combo: usize },
    /// [`ComboKeyIdent::Coord`] is outside the keyboard.
    ComboCoord { combo: usize, x: u8, y: u8 },
    /// [`ComboAction::SetLayout`] points to a missing layout.
    ComboLayoutIndex { combo: usize, index: u8 },
    /// [`ComboAction::BtSelect`] points to a missing BLE profile.
    ComboBtProfileIndex { combo: usize, index: u8 },
    /// [`ComboKeyIdent::Action`] is [`KeyAction::Transparent`]: a transparent key resolves to the action of
    /// the layer below, so it never matches.
    ComboTransparentKey { combo: usize },
    /// [`ComboKeyIdent::Action`] is invalid, like it would be on a key.
    ComboKey { combo: usize, error: ActionError },
}

/// Why a [`KeyAction`] is invalid wherever it is, see [`ValidationError::Key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// Layer action points to a missing layout.
    LayerIndex(u8),
    /// [`KeyAction::BtSelect`] points to a missing BLE profile, see [`BT_PROFILES`].
    BtProfileIndex(u8),
    /// [`KeyAction::ModTap`] has no modifiers.
    ModTapWithoutMods,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ActionError::LayerIndex(index) =>
                write!(f, "points to missing layout {index}"),
            ActionError::BtProfileIndex(index) =>
                write!(f, "selects BLE profile {index}, there are {BT_PROFILES} (0..={})", BT_PROFILES - 1),
            ActionError::ModTapWithoutMods =>
                write!(f, "is a mod-tap without modifiers"),
        }
    }
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
            ValidationError::JoystickFn { layout } =>
                write!(f, "layout {layout}: Fn can't be on a joystick button"),
            ValidationError::Key { layout, x, y, error } =>
                write!(f, "layout {layout}: key at ({x}, {y}) {error}"),
            ValidationError::Joystick { layout, button, error } =>
                write!(f, "layout {layout}: joystick {} button {error}", button.name()),
            ValidationError::Setting { name, value, min, max } =>
                write!(f, "settings: {name} is {value}, must be {min}..={max}"),
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
            ValidationError::ComboBtProfileIndex { combo, index } =>
                write!(f, "combo {combo}: selects BLE profile {index}, there are {BT_PROFILES} (0..={})", BT_PROFILES - 1),
            ValidationError::ComboTransparentKey { combo } =>
                write!(f, "combo {combo}: a Transparent key never matches, use the key's position or action"),
            ValidationError::ComboKey { combo, error } =>
                write!(f, "combo {combo}: key action {error}"),
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
        game[2][0] = KeyAction::ModTap { mods: KeyMod::LeftCtrl | KeyMod::LeftShift, tap: Key::A };
        game[2][1] = KeyAction::LayerTap { layer: 1, tap: Key::Space };
        game[2][2] = KeyAction::Momentary(1);
        game[2][3] = KeyAction::Toggle(0);
        game[2][4] = KeyAction::To(2);
        game[2][5] = KeyAction::Transparent;
        game[0][0] = KeyAction::BtSelect(3);
        game[0][1] = KeyAction::BtNext;
        game[0][2] = KeyAction::BtPrev;
        game[0][3] = KeyAction::BtClear;

        Config {
            version: CONFIG_VERSION,
            width: 10,
            height: 5,
            layouts: vec![
                Layout { name: "MAIN".to_string(), rows: main, joystick: JoystickButtons::default() },
                Layout {
                    name: "FUNCTION".to_string(),
                    rows: func,
                    joystick: JoystickButtons { stick: KeyAction::None, ..JoystickButtons::default() },
                },
                Layout {
                    name: "GAME".to_string(),
                    rows: game,
                    joystick: JoystickButtons {
                        left: KeyAction::MouseKey(MouseKey::Left),
                        stick: KeyAction::Key(Key::Space),
                        right: KeyAction::Momentary(1),
                    },
                },
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
                Combo {
                    keys: vec![ComboKey { ident: ComboKeyIdent::Coord { x: 9, y: 3 }, count: 2 }],
                    action: ComboAction::BtSelect(1),
                },
            ],
            settings: Settings { tapping_term_ms: 180, require_prior_idle_ms: 0 },
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

    /// [`sample`] without what version 2 didn't have.
    fn sample_v2() -> v2::Config {
        let mut config = sample();
        for layout in &mut config.layouts {
            for action in layout.rows.iter_mut().flatten() {
                if matches!(action, KeyAction::ModTap { .. } | KeyAction::LayerTap { .. } | KeyAction::Momentary(_)
                    | KeyAction::Toggle(_) | KeyAction::To(_) | KeyAction::Transparent | KeyAction::BtSelect(_)
                    | KeyAction::BtNext | KeyAction::BtPrev | KeyAction::BtClear) {
                    *action = KeyAction::None;
                }
            }
            if matches!(layout.joystick.right, KeyAction::Momentary(_)) {
                layout.joystick.right = KeyAction::None;
            }
        }
        let Config { width, height, layouts, mut combos, .. } = config;
        combos.retain(|combo| !matches!(combo.action, ComboAction::BtSelect(_)));
        v2::Config { version: 2, width, height, layouts, combos }
    }

    #[test]
    fn version_2_is_upgraded() {
        let old = sample_v2();
        let upgraded = Config::from_bytes(&postcard::to_allocvec(&old).unwrap()).unwrap();
        assert_eq!(upgraded.version, CONFIG_VERSION);
        assert_eq!(upgraded.settings, Settings::default());
        assert_eq!(upgraded.layouts, old.layouts);
        assert_eq!(upgraded.combos, old.combos);
        assert_eq!(upgraded.validate_for(10, 5), Ok(()));
    }

    #[test]
    fn new_actions_keep_old_indices() {
        // postcard writes the variant index first: the old variants must not move.
        assert_eq!(postcard::to_allocvec(&KeyAction::None).unwrap(), [8]);
        assert_eq!(postcard::to_allocvec(&KeyAction::Transparent).unwrap(), [9]);
        assert_eq!(postcard::to_allocvec(&KeyAction::Momentary(3)).unwrap(), [10, 3]);
        assert_eq!(postcard::to_allocvec(&KeyAction::LayerTap { layer: 1, tap: Key::A }).unwrap(), [14, 1, 4]);
        assert_eq!(postcard::to_allocvec(&KeyAction::BtSelect(2)).unwrap(), [15, 2]);
        assert_eq!(postcard::to_allocvec(&KeyAction::BtClear).unwrap(), [18]);
        assert_eq!(postcard::to_allocvec(&ComboAction::Unpair).unwrap(), [2]);
        assert_eq!(postcard::to_allocvec(&ComboAction::ResetConfig).unwrap(), [3]);
        assert_eq!(postcard::to_allocvec(&ComboAction::BtSelect(1)).unwrap(), [4, 1]);
        assert_eq!(postcard::to_allocvec(&ComboAction::BtClearAll).unwrap(), [8]);
    }

    #[test]
    fn version_1_is_upgraded() {
        let config = sample();
        let old = v1::Config {
            version: 1,
            width: config.width,
            height: config.height,
            layouts: config.layouts.iter().map(|l| v1::Layout { name: l.name.clone(), rows: l.rows.clone() }).collect(),
            combos: config.combos.clone(),
        };
        let upgraded = Config::from_bytes(&postcard::to_allocvec(&old).unwrap()).unwrap();
        assert_eq!(upgraded.version, CONFIG_VERSION);
        assert_eq!(upgraded.settings, Settings::default());
        assert!(upgraded.layouts.iter().all(|l| l.joystick == JoystickButtons::default()));
        assert_eq!(upgraded.layouts.iter().map(|l| &l.rows).collect::<Vec<_>>(), config.layouts.iter().map(|l| &l.rows).collect::<Vec<_>>());
        assert_eq!(upgraded.validate_for(10, 5), Ok(()));
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
            version = 3
            width = 2
            height = 1

            [settings]
            tapping_term_ms = 250

            [[layouts]]
            name = "MAIN"
            rows = [
                [{ Key = "A" }, { Fn = { temporary = 0, permanent = 0 } }],
            ]

            [[layouts]]
            name = "MEDIA"
            rows = [
                ["Transparent", { ModTap = { mods = "LeftCtrl", tap = "F" } }],
            ]
            joystick = { left = { Toggle = 0 }, stick = { Media = "Mute" }, right = { MouseKey = "Right | Middle" } }

            [[combos]]
            keys = [{ ident = { Coord = { x = 1, y = 0 } }, count = 0 }, { ident = { Action = { Key = "A" } }, count = 2 }]
            action = { SetLayout = 1 }

            [[combos]]
            keys = [{ ident = { Action = "None" }, count = 1 }]
            action = "ResetConfig"

            [[combos]]
            keys = [{ ident = { Action = "BtNext" }, count = 2 }]
            action = { BtSelect = 3 }
        "#;
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.validate(), Ok(()));
        assert_eq!(config.settings, Settings { tapping_term_ms: 250, ..Settings::default() });
        assert_eq!(
            config.layouts[1].rows[0],
            [KeyAction::Transparent, KeyAction::ModTap { mods: KeyMod::LeftCtrl, tap: Key::F }]
        );
        assert_eq!(config.layouts[0].joystick, JoystickButtons::default());
        assert_eq!(
            config.layouts[1].joystick,
            JoystickButtons {
                left: KeyAction::Toggle(0),
                stick: KeyAction::Media(MediaKey::Mute),
                right: KeyAction::MouseKey(MouseKey::Right | MouseKey::Middle),
            }
        );
        assert_eq!(config.combos[0].action, ComboAction::SetLayout(1));
        assert_eq!(config.combos[1].action, ComboAction::ResetConfig);
        assert_eq!(config.combos[2].keys[0].ident, ComboKeyIdent::Action(KeyAction::BtNext));
        assert_eq!(config.combos[2].action, ComboAction::BtSelect(3));
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
            invalid(|c| c.layouts[1].joystick.left = KeyAction::Fn { temporary: 0, permanent: 0 }),
            ValidationError::JoystickFn { layout: 1 }
        );
        assert_eq!(
            invalid(|c| c.layouts[0].rows[1][3] = KeyAction::LayerTap { layer: 3, tap: Key::A }),
            ValidationError::Key { layout: 0, x: 3, y: 1, error: ActionError::LayerIndex(3) }
        );
        assert_eq!(
            invalid(|c| c.layouts[2].rows[0][0] = KeyAction::To(8)),
            ValidationError::Key { layout: 2, x: 0, y: 0, error: ActionError::LayerIndex(8) }
        );
        assert_eq!(
            invalid(|c| c.layouts[1].joystick.stick = KeyAction::Toggle(3)),
            ValidationError::Joystick { layout: 1, button: JoystickButton::Stick, error: ActionError::LayerIndex(3) }
        );
        assert_eq!(
            invalid(|c| c.settings.tapping_term_ms = 10),
            ValidationError::Setting { name: "tapping_term_ms", value: 10, min: 50, max: 1000 }
        );
        assert_eq!(
            invalid(|c| c.settings.require_prior_idle_ms = 5000),
            ValidationError::Setting { name: "require_prior_idle_ms", value: 5000, min: 0, max: 1000 }
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
            invalid(|c| c.layouts[1].rows[2][3] = KeyAction::BtSelect(BT_PROFILES as u8)),
            ValidationError::Key { layout: 1, x: 3, y: 2, error: ActionError::BtProfileIndex(BT_PROFILES as u8) }
        );
        assert_eq!(
            invalid(|c| c.layouts[0].joystick.right = KeyAction::BtSelect(200)),
            ValidationError::Joystick { layout: 0, button: JoystickButton::Right, error: ActionError::BtProfileIndex(200) }
        );
        assert_eq!(
            invalid(|c| c.combos[1].action = ComboAction::BtSelect(BT_PROFILES as u8)),
            ValidationError::ComboBtProfileIndex { combo: 1, index: BT_PROFILES as u8 }
        );
        assert_eq!(
            invalid(|c| c.combos[1].keys[0].ident = ComboKeyIdent::Action(KeyAction::Transparent)),
            ValidationError::ComboTransparentKey { combo: 1 }
        );
        assert_eq!(
            invalid(|c| c.combos[1].keys[0].ident = ComboKeyIdent::Action(KeyAction::Momentary(3))),
            ValidationError::ComboKey { combo: 1, error: ActionError::LayerIndex(3) }
        );
        assert_eq!(
            invalid(|c| c.combos[0].keys[0].ident = ComboKeyIdent::Action(KeyAction::Fn { temporary: 1, permanent: 7 })),
            ValidationError::ComboKey { combo: 0, error: ActionError::LayerIndex(7) }
        );
        assert_eq!(
            invalid(|c| c.combos[2].keys[0].ident = ComboKeyIdent::Action(KeyAction::LayerTap { layer: 9, tap: Key::A })),
            ValidationError::ComboKey { combo: 2, error: ActionError::LayerIndex(9) }
        );
        assert_eq!(
            invalid(|c| c.combos[1].keys[0].ident = ComboKeyIdent::Action(KeyAction::BtSelect(BT_PROFILES as u8))),
            ValidationError::ComboKey { combo: 1, error: ActionError::BtProfileIndex(BT_PROFILES as u8) }
        );
        let no_mods = KeyAction::ModTap { mods: KeyMod::empty(), tap: Key::A };
        assert_eq!(
            invalid(|c| c.layouts[1].rows[2][3] = no_mods),
            ValidationError::Key { layout: 1, x: 3, y: 2, error: ActionError::ModTapWithoutMods }
        );
        assert_eq!(
            invalid(|c| c.layouts[2].joystick.stick = no_mods),
            ValidationError::Joystick { layout: 2, button: JoystickButton::Stick, error: ActionError::ModTapWithoutMods }
        );
        assert_eq!(
            invalid(|c| c.combos[0].keys[0].ident = ComboKeyIdent::Action(no_mods)),
            ValidationError::ComboKey { combo: 0, error: ActionError::ModTapWithoutMods }
        );
        // In range: valid.
        let mut config = sample();
        config.combos[1].keys[0].ident = ComboKeyIdent::Action(KeyAction::To(2));
        config.combos[2].keys[0].ident = ComboKeyIdent::Action(KeyAction::BtSelect(BT_PROFILES as u8 - 1));
        config.validate().unwrap();
        assert_eq!(
            sample().validate_for(12, 5),
            Err(ValidationError::WrongKeyboardSize { expected_width: 12, expected_height: 5, width: 10, height: 5 })
        );
    }
}
