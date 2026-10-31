//! [`Config::validate`]: the checks of a decoded config, wherever it came from.

use core::fmt;

use super::{
    Combo, ComboAction, ComboKeyIdent, Config, JoystickButton, KeyAction, Layout, Settings, BT_PROFILES,
    CONFIG_VERSION, MAX_COMBOS, MAX_COMBO_KEYS, MAX_LAYOUTS, MAX_LAYOUT_NAME_LEN,
};

impl Config {
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
            ("combo_term_ms", self.settings.combo_term_ms, Settings::COMBO_TERM_MS),
            ("debounce_ms", self.settings.debounce_ms, Settings::DEBOUNCE_MS),
            ("sleep_connected_s", self.settings.sleep_connected_s, Settings::SLEEP_S),
            ("sleep_disconnected_s", self.settings.sleep_disconnected_s, Settings::SLEEP_S),
            ("joystick_speed", self.settings.joystick_speed, Settings::JOYSTICK_SPEED),
            ("joystick_ramp_ms", self.settings.joystick_ramp_ms, Settings::JOYSTICK_RAMP_MS),
            ("display_off_s", self.settings.display_off_s, Settings::DISPLAY_OFF_S),
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
        // Its hold would do nothing, yet it delays the taps.
        if matches!(action, KeyAction::ModTap { mods, .. } if mods.is_empty()) {
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
        if value.is_chord() {
            if value.keys.len() < 2 {
                return Err(ValidationError::ChordTooFewKeys { combo });
            }
            if value.keys.iter().any(|key| key.count != 0) {
                return Err(ValidationError::ChordKeyCount { combo });
            }
            // A key is pressed once, a chord with the same position twice would never be complete.
            let duplicate = value.keys.iter().enumerate().any(|(index, key)| {
                matches!(key.ident, ComboKeyIdent::Coord { .. })
                    && value.keys[..index].iter().any(|earlier| earlier.ident == key.ident)
            });
            if duplicate {
                return Err(ValidationError::ChordDuplicateKey { combo });
            }
        } else if value.keys.iter().all(|key| key.count == 0) {
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
            ComboAction::Action(
                KeyAction::Fn { .. } | KeyAction::Transparent | KeyAction::ModTap { .. } | KeyAction::LayerTap { .. },
            ) => return Err(ValidationError::ChordActionKind { combo }),
            ComboAction::Action(action) => {
                self.check_action(&action).map_err(|error| ValidationError::ChordAction { combo, error })?;
            }
            _ => {}
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
    /// A chord ([`ComboAction::Action`]) has a single key.
    ChordTooFewKeys { combo: usize },
    /// A key of a chord has a number of presses, chord keys are only pressed together.
    ChordKeyCount { combo: usize },
    /// A chord has the same key position twice, it could never be pressed.
    ChordDuplicateKey { combo: usize },
    /// The action of a chord is [`KeyAction::Fn`], [`KeyAction::Transparent`] or a tap-hold one.
    ChordActionKind { combo: usize },
    /// The action of a chord is invalid, like it would be on a key.
    ChordAction { combo: usize, error: ActionError },
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
            ValidationError::ChordTooFewKeys { combo } =>
                write!(f, "combo {combo}: a chord needs at least 2 keys"),
            ValidationError::ChordKeyCount { combo } =>
                write!(f, "combo {combo}: chord keys are pressed together, they can't have a number of presses"),
            ValidationError::ChordDuplicateKey { combo } =>
                write!(f, "combo {combo}: a chord has the same key twice, it could never be pressed"),
            ValidationError::ChordActionKind { combo } =>
                write!(f, "combo {combo}: a chord can't send Fn, Transparent, a mod-tap or a layer-tap"),
            ValidationError::ChordAction { combo, error } =>
                write!(f, "combo {combo}: chord action {error}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ValidationError {}
