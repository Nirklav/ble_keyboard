//! Keyboard configuration: layouts (layers) and key combinations (special ones and chords).
//!
//! The same [`Config`] is used everywhere:
//! - on the wire (HID config reports, see [`crate::hid`]) and in the firmware flash it is
//!   encoded with `postcard` ([`Config::to_bytes`], [`Config::from_bytes`]);
//! - in the desktop app it is a TOML file with key names.
//!
//! Compatibility: enum variants are only appended (postcard encodes the variant index), new [`Settings`] are
//! appended to their end (see [`Config::from_bytes`]); any other change to the layout of these types bumps
//! [`CONFIG_VERSION`]. The previous layouts are kept (see [`v1`], [`v2`], [`v3`]) and upgraded on decode,
//! so saved configs survive a firmware update.

use alloc::string::String;
use alloc::vec::Vec;
use core::ops::RangeInclusive;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::keys::{Key, KeyMod, MediaKey, MouseKey};

/// Format version of [`Config`], stored in [`Config::version`].
pub const CONFIG_VERSION: u16 = 4;

/// Oldest [`Config::version`] [`Config::from_bytes`] still reads (and upgrades).
pub const OLDEST_CONFIG_VERSION: u16 = 1;

pub const MAX_LAYOUTS: usize = 8;

pub const MAX_COMBOS: usize = 16;

pub const MAX_COMBO_KEYS: usize = 8;

/// Max length of a layout name in characters: the 128 px wide display status shows it in the 7x14 font
/// beside the BLE profile.
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
    /// Moves the mouse cursor while held: `dx`, `dy` pixels per [`crate::motion::TRAVEL_UNIT_MS`] (10 ms),
    /// whatever the report rate is.
    MouseTravel { dx: i8, dy: i8 },
    /// Scrolls the mouse wheel while held: `dy` steps per second, positive is up.
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
}

/// Global settings: key processing, sleep, display, joystick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// A [`KeyAction::ModTap`] or [`KeyAction::LayerTap`] held this long is a hold.
    /// Released earlier it's a tap, unless another key was pressed and released meanwhile.
    pub tapping_term_ms: u16,
    /// A tap-hold key pressed this soon after the press of a typed key is a tap at once:
    /// no misfires while typing fast. 0 turns it off.
    pub require_prior_idle_ms: u16,
    /// The keys of a chord ([`ComboAction::Action`]) must all be pressed within this time after the first one.
    pub combo_term_ms: u16,
    /// A key change is ignored for this long after the previous one: contact bounce.
    pub debounce_ms: u16,
    /// Deep sleep after this many seconds without activity while a host is connected.
    pub sleep_connected_s: u16,
    /// Deep sleep after this many seconds without activity while no host is connected.
    pub sleep_disconnected_s: u16,
    /// Cursor speed at full joystick deflection, in pixels per second.
    pub joystick_speed: u16,
    /// The cursor reaches the speed of the deflection in about this many ms, 0 is at once.
    pub joystick_ramp_ms: u16,
    pub joystick_invert_x: bool,
    pub joystick_invert_y: bool,
    /// The display status goes off after this many seconds without key presses: an OLED burns in.
    pub display_off_s: u16,
}

impl Settings {
    /// Allowed [`Settings::tapping_term_ms`].
    pub const TAPPING_TERM_MS: RangeInclusive<u16> = 50..=1000;
    /// Allowed [`Settings::require_prior_idle_ms`].
    pub const REQUIRE_PRIOR_IDLE_MS: RangeInclusive<u16> = 0..=1000;
    /// Allowed [`Settings::combo_term_ms`].
    pub const COMBO_TERM_MS: RangeInclusive<u16> = 10..=200;
    /// Allowed [`Settings::debounce_ms`].
    pub const DEBOUNCE_MS: RangeInclusive<u16> = 1..=50;
    /// Allowed [`Settings::sleep_connected_s`] and [`Settings::sleep_disconnected_s`]: the keyboard goes idle
    /// after 15 s, so it can't sleep sooner; up to 10 hours.
    pub const SLEEP_S: RangeInclusive<u16> = 15..=36000;
    /// Allowed [`Settings::joystick_speed`].
    pub const JOYSTICK_SPEED: RangeInclusive<u16> = 100..=6000;
    /// Allowed [`Settings::joystick_ramp_ms`].
    pub const JOYSTICK_RAMP_MS: RangeInclusive<u16> = 0..=500;
    /// Allowed [`Settings::display_off_s`]: longer than a pop-up, up to an hour.
    pub const DISPLAY_OFF_S: RangeInclusive<u16> = 10..=3600;

    /// Decodes postcard `bytes` that may end before the last settings, those are the defaults then.
    /// Every field is listed: one added to the struct doesn't compile until it's read here, in its order.
    fn from_bytes(mut bytes: &[u8]) -> Result<Settings, postcard::Error> {
        let mut settings = Settings::default();
        let Settings {
            tapping_term_ms,
            require_prior_idle_ms,
            combo_term_ms,
            debounce_ms,
            sleep_connected_s,
            sleep_disconnected_s,
            joystick_speed,
            joystick_ramp_ms,
            joystick_invert_x,
            joystick_invert_y,
            display_off_s,
        } = &mut settings;
        take(&mut bytes, tapping_term_ms)?;
        take(&mut bytes, require_prior_idle_ms)?;
        take(&mut bytes, combo_term_ms)?;
        take(&mut bytes, debounce_ms)?;
        take(&mut bytes, sleep_connected_s)?;
        take(&mut bytes, sleep_disconnected_s)?;
        take(&mut bytes, joystick_speed)?;
        take(&mut bytes, joystick_ramp_ms)?;
        take(&mut bytes, joystick_invert_x)?;
        take(&mut bytes, joystick_invert_y)?;
        take(&mut bytes, display_off_s)?;
        Ok(settings)
    }
}

/// Decodes `field` from the start of `bytes` and moves past it; none left keeps it as it is.
fn take<T: DeserializeOwned>(bytes: &mut &[u8], field: &mut T) -> Result<(), postcard::Error> {
    if !bytes.is_empty() {
        let (value, rest) = postcard::take_from_bytes(bytes)?;
        *field = value;
        *bytes = rest;
    }
    Ok(())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tapping_term_ms: 200,
            require_prior_idle_ms: 150,
            combo_term_ms: 50,
            debounce_ms: 5,
            sleep_connected_s: 300,
            sleep_disconnected_s: 60,
            joystick_speed: 1800,
            joystick_ramp_ms: 40,
            joystick_invert_x: false,
            joystick_invert_y: false,
            display_off_s: 120,
        }
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
    /// In the order they are shown.
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

/// Key combination: a special one or a chord.
///
/// A special combination watches the keys without changing what they do: once the held keys are held and
/// the others pressed their number of times, the action runs.
///
/// A chord ([`ComboAction::Action`]) replaces its keys: pressed together, within [`Settings::combo_term_ms`],
/// they do the action instead of what each of them does, see [`crate::keymap`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combo {
    /// Keys that make the combination, 1..=[`MAX_COMBO_KEYS`], at least one with `count > 0`;
    /// a chord has at least 2 keys, all with `count == 0`.
    pub keys: Vec<ComboKey>,
    pub action: ComboAction,
}

impl Combo {
    /// [`ComboAction::Action`]: the keys are a chord.
    pub fn is_chord(&self) -> bool {
        matches!(self.action, ComboAction::Action(_))
    }
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

impl ComboKeyIdent {
    /// Whether a key at `coord` (`None` for a joystick button) pressed as `action` is this key.
    pub fn matches(&self, coord: Option<(u8, u8)>, action: &KeyAction) -> bool {
        match self {
            ComboKeyIdent::Coord { x, y } => coord == Some((*x, *y)),
            ComboKeyIdent::Action(ident) => ident == action,
        }
    }
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
    /// Chord: the keys pressed together do this action while they are held, instead of their own ones.
    /// Not [`KeyAction::Fn`], [`KeyAction::Transparent`] or a tap-hold action.
    Action(KeyAction),
}

impl ComboAction {
    /// Key action of a chord, [`ComboAction::Action`].
    pub fn key_action(&self) -> Option<&KeyAction> {
        match self {
            ComboAction::Action(action) => Some(action),
            _ => None,
        }
    }

    /// Like [`KeyAction::map_layers`], for every layout index the action points to.
    pub fn map_layers(&mut self, mut map: impl FnMut(u8) -> u8) {
        match self {
            ComboAction::SetLayout(index) => *index = map(*index),
            ComboAction::Action(action) => action.map_layers(map),
            _ => {}
        }
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
    ///
    /// [`Config::settings`] are the last field: a newer app or firmware appends settings to their end without
    /// a new version. An older one stops before them (postcard ignores what's left of the input), here
    /// the settings an older one left out are the defaults.
    pub fn from_bytes(bytes: &[u8]) -> Result<Config, postcard::Error> {
        // `version` is the first field, so it can be read before the layout of the rest is known.
        let (version, _) = postcard::take_from_bytes::<u16>(bytes)?;
        if version != CONFIG_VERSION {
            return Config::decode(version, &Postcard(bytes));
        }
        // Encoded like a struct of the same fields.
        let ((version, width, height, layouts, combos), settings) =
            postcard::take_from_bytes::<(u16, u8, u8, Vec<Layout>, Vec<Combo>)>(bytes)?;
        let settings = Settings::from_bytes(settings)?;
        Ok(Config { version, width, height, layouts, combos, settings })
    }

    /// Decodes a config of `version` from `input`, an older version is upgraded to [`CONFIG_VERSION`].
    /// Doesn't [`validate`](Config::validate).
    pub fn decode<F: Format>(version: u16, input: &F) -> Result<Config, F::Error> {
        match version {
            1 => input.decode::<v1::Config>().map(Config::from),
            2 => input.decode::<v2::Config>().map(Config::from),
            3 => input.decode::<v3::Config>().map(Config::from),
            _ => input.decode(),
        }
    }
}

/// Encoding a config is decoded from by [`Config::decode`], in any of its versions: postcard, the TOML of the apps.
pub trait Format {
    type Error;

    /// Decodes the whole input as `T`.
    fn decode<T: DeserializeOwned>(&self) -> Result<T, Self::Error>;
}

/// Postcard bytes, see [`Config::from_bytes`].
struct Postcard<'a>(&'a [u8]);

impl Format for Postcard<'_> {
    type Error = postcard::Error;

    fn decode<T: DeserializeOwned>(&self) -> Result<T, postcard::Error> {
        postcard::from_bytes(self.0)
    }
}

pub mod v1;
pub mod v2;
pub mod v3;
mod validate;

pub use validate::{ActionError, ValidationError};

#[cfg(test)]
pub(crate) mod tests;
