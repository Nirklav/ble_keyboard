//! Short texts for keys and actions shown on the grid buttons and in pickers.

use std::ops::Range;

use protocol::config::{ComboAction, ComboKey, ComboKeyIdent, KeyAction, Layout};
use protocol::keys::{Key, KeyMod, MediaKey, MouseKey};

/// Every [`Key`], in usage ID order (`KEYS[i].usage() == i`).
#[rustfmt::skip]
pub const KEYS: [Key; 116] = {
    use Key::*;
    [
        None, ErrorRollOver, PostFail, ErrorUndefined,
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
        Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0,
        Enter, Escape, Backspace, Tab, Space, Minus, Equal, LeftBracket, RightBracket, Backslash, NonUsHash, Semicolon, Apostrophe, Grave, Comma, Dot, Slash, CapsLock,
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
        PrintScreen, ScrollLock, Pause, Insert, Home, PageUp, Delete, End, PageDown, Right, Left, Down, Up,
        NumLock, KpSlash, KpAsterisk, KpMinus, KpPlus, KpEnter, Kp1, Kp2, Kp3, Kp4, Kp5, Kp6, Kp7, Kp8, Kp9, Kp0, KpDot,
        NonUsBackslash, Application, Power, KpEqual,
        F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
    ]
};

/// Groups of [`KEYS`] for the key picker: the title and the index ranges in `KEYS`.
/// Every key is in one group; the HID error codes are last, nobody needs them on a key.
#[allow(clippy::single_range_in_vec_init)] // Slices of ranges, not ranges.
pub const KEY_GROUPS: [(&str, &[Range<usize>]); 8] = [
    ("", &[0..1]),
    ("Letters", &[4..30]),
    ("Digits", &[30..40]),
    ("Basic and punctuation", &[40..58]),
    ("Function keys", &[58..70, 104..116]),
    ("Navigation and editing", &[70..83]),
    ("Keypad", &[83..100, 103..104]),
    ("Other", &[100..103, 1..4]),
];

/// Modifier bits with their short names, in report bit order.
pub const MODIFIERS: [(KeyMod, &str); 8] = [
    (KeyMod::LeftCtrl, "LCtrl"),
    (KeyMod::LeftShift, "LShift"),
    (KeyMod::LeftAlt, "LAlt"),
    (KeyMod::LeftGui, "LGui"),
    (KeyMod::RightCtrl, "RCtrl"),
    (KeyMod::RightShift, "RShift"),
    (KeyMod::RightAlt, "RAlt"),
    (KeyMod::RightGui, "RGui"),
];

/// Mouse buttons with their names and short names.
pub const MOUSE_BUTTONS: [(MouseKey, &str, &str); 5] = [
    (MouseKey::Left, "Left", "L"),
    (MouseKey::Right, "Right", "R"),
    (MouseKey::Middle, "Middle", "M"),
    (MouseKey::Back, "Back", "Back"),
    (MouseKey::Forward, "Forward", "Fwd"),
];

/// Identifier of the key as written in the TOML file, e.g. `Num1`.
pub fn key_name(key: Key) -> String {
    format!("{key:?}")
}

/// Short label of a key, e.g. `1` for `Num1`, `Bksp` for `Backspace`.
pub fn key_label(key: Key) -> String {
    let label = match key {
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::Num0 => "0",
        Key::Escape => "Esc",
        Key::Backspace => "Bksp",
        Key::Minus => "-",
        Key::Equal => "=",
        Key::LeftBracket => "[",
        Key::RightBracket => "]",
        Key::Backslash => "\\",
        Key::NonUsHash => "# (ISO)",
        Key::Semicolon => ";",
        Key::Apostrophe => "'",
        Key::Grave => "`",
        Key::Comma => ",",
        Key::Dot => ".",
        Key::Slash => "/",
        Key::CapsLock => "Caps",
        Key::PrintScreen => "PrtSc",
        Key::ScrollLock => "ScrLk",
        Key::Insert => "Ins",
        Key::PageUp => "PgUp",
        Key::Delete => "Del",
        Key::PageDown => "PgDn",
        Key::NumLock => "NumLk",
        Key::KpSlash => "Kp /",
        Key::KpAsterisk => "Kp *",
        Key::KpMinus => "Kp -",
        Key::KpPlus => "Kp +",
        Key::KpEnter => "Kp Enter",
        Key::Kp1 => "Kp 1",
        Key::Kp2 => "Kp 2",
        Key::Kp3 => "Kp 3",
        Key::Kp4 => "Kp 4",
        Key::Kp5 => "Kp 5",
        Key::Kp6 => "Kp 6",
        Key::Kp7 => "Kp 7",
        Key::Kp8 => "Kp 8",
        Key::Kp9 => "Kp 9",
        Key::Kp0 => "Kp 0",
        Key::KpDot => "Kp .",
        Key::KpEqual => "Kp =",
        Key::NonUsBackslash => "\\ (ISO)",
        Key::Application => "Menu",
        _ => return key_name(key),
    };
    label.to_string()
}

/// Character of `key` with Shift on the US layout, e.g. `!` for `Num1`; `None` for letters and the rest.
fn shifted_label(key: Key) -> Option<&'static str> {
    let label = match key {
        Key::Num1 => "!",
        Key::Num2 => "@",
        Key::Num3 => "#",
        Key::Num4 => "$",
        Key::Num5 => "%",
        Key::Num6 => "^",
        Key::Num7 => "&",
        Key::Num8 => "*",
        Key::Num9 => "(",
        Key::Num0 => ")",
        Key::Minus => "_",
        Key::Equal => "+",
        Key::LeftBracket => "{",
        Key::RightBracket => "}",
        Key::Backslash => "|",
        Key::Semicolon => ":",
        Key::Apostrophe => "\"",
        Key::Grave => "~",
        Key::Comma => "<",
        Key::Dot => ">",
        Key::Slash => "?",
        _ => return None,
    };
    Some(label)
}

/// Modifiers joined with `+`, e.g. `LCtrl+LShift`; `-` if empty.
pub fn mods_label(mods: KeyMod) -> String {
    if mods.is_empty() {
        return "-".to_string();
    }
    let names: Vec<&str> = MODIFIERS.iter().filter(|(m, _)| mods.contains(*m)).map(|(_, name)| *name).collect();
    names.join("+")
}

/// Modifiers as a key prefix, left ones without the side: `Shift`, `Ctrl+Alt`, `RShift`.
fn mods_prefix(mods: KeyMod) -> String {
    let names: Vec<&str> = MODIFIERS
        .iter()
        .filter(|(m, _)| mods.contains(*m))
        .map(|(_, name)| name.strip_prefix('L').unwrap_or(name))
        .collect();
    names.join("+")
}

/// Short label of a media key, e.g. `Vol+`.
pub fn media_label(key: MediaKey) -> &'static str {
    match key {
        MediaKey::NextTrack => "Next",
        MediaKey::PreviousTrack => "Prev",
        MediaKey::Stop => "Stop",
        MediaKey::PlayPause => "Play",
        MediaKey::Mute => "Mute",
        MediaKey::VolumeUp => "Vol+",
        MediaKey::VolumeDown => "Vol-",
        MediaKey::Calculator => "Calc",
    }
}

/// Mouse buttons, e.g. `Mouse L`, `Mouse L+R`.
pub fn mouse_label(buttons: MouseKey) -> String {
    let names: Vec<&str> =
        MOUSE_BUTTONS.iter().filter(|(b, ..)| buttons.contains(*b)).map(|(_, _, short)| *short).collect();
    if names.is_empty() { "Mouse -".to_string() } else { format!("Mouse {}", names.join("+")) }
}

/// Short label of [`KeyAction::Transparent`]; ASCII, the default fonts have no triangle.
pub const TRANSPARENT_LABEL: &str = "TRNS";

/// Short label of an action for a key button, empty for [`KeyAction::None`].
pub fn action_label(action: &KeyAction) -> String {
    match *action {
        KeyAction::Key(key) => key_label(key),
        KeyAction::KeyWithMod(key, mods) if mods.is_empty() => key_label(key),
        KeyAction::KeyWithMod(key, KeyMod::LeftShift) if shifted_label(key).is_some() => {
            shifted_label(key).unwrap_or_default().to_string()
        }
        KeyAction::KeyWithMod(key, mods) => format!("{}+{}", mods_prefix(mods), key_label(key)),
        KeyAction::Mod(mods) => mods_label(mods),
        KeyAction::Media(key) => media_label(key).to_string(),
        KeyAction::MouseKey(buttons) => mouse_label(buttons),
        KeyAction::MouseTravel { dx, dy } => format!("Move {dx},{dy}"),
        KeyAction::MouseScroll { dy } => format!("Scroll {dy:+}"),
        KeyAction::Fn { temporary, permanent } => format!("Fn {temporary}/{permanent}"),
        KeyAction::None => String::new(),
        KeyAction::Transparent => TRANSPARENT_LABEL.to_string(),
        KeyAction::Momentary(layer) => format!("MO {layer}"),
        KeyAction::Toggle(layer) => format!("TG {layer}"),
        KeyAction::To(layer) => format!("TO {layer}"),
        // Invalid (see `ActionError::ModTapWithoutMods`), shown as such rather than as a plain key.
        KeyAction::ModTap { mods, tap } if mods.is_empty() => format!("-/{}", key_label(tap)),
        KeyAction::ModTap { mods, tap } => format!("{}/{}", mods_prefix(mods), key_label(tap)),
        KeyAction::LayerTap { layer, tap } => format!("LT{layer} {}", key_label(tap)),
        KeyAction::BtSelect(profile) => bt_profile_name(profile),
        KeyAction::BtNext => "BT >".to_string(),
        KeyAction::BtPrev => "BT <".to_string(),
        KeyAction::BtClear => "BT clr".to_string(),
    }
}

/// BLE profile as the keyboard shows it, 1-based: `BT 1` for profile 0.
pub fn bt_profile_name(profile: u8) -> String {
    format!("BT {}", profile as u16 + 1)
}

/// Longer description of an action, for tooltips.
pub fn action_description(action: &KeyAction) -> String {
    match *action {
        KeyAction::Key(key) => format!("Key {}", key_name(key)),
        KeyAction::KeyWithMod(key, mods) => {
            format!("Key {} with {}", key_name(key), mods_label(mods))
        }
        KeyAction::Mod(mods) => format!("Modifiers {}", mods_label(mods)),
        KeyAction::Media(key) => format!("Media: {key}"),
        KeyAction::MouseKey(buttons) => format!("{} button(s)", mouse_label(buttons)),
        KeyAction::MouseTravel { dx, dy } => {
            format!("Move the mouse cursor by ({dx}, {dy}) per report while held")
        }
        KeyAction::MouseScroll { dy } => {
            format!("Scroll the mouse wheel by {dy} per report while held")
        }
        KeyAction::Fn { temporary, permanent } => {
            format!("Fn: layout {temporary} while held, layout {permanent} after a double tap")
        }
        KeyAction::None => "Nothing".to_string(),
        KeyAction::Transparent => "Transparent: does what this key does in the layer below".to_string(),
        KeyAction::Momentary(layer) => format!("MO: layout {layer} while held"),
        KeyAction::Toggle(layer) => format!("TG: turns layout {layer} on or off"),
        KeyAction::To(layer) => format!("TO: switches to layout {layer}, turns the other layers off"),
        KeyAction::ModTap { mods, tap } => format!("Mod-tap: tap {}, hold {}", key_name(tap), mods_label(mods)),
        KeyAction::LayerTap { layer, tap } => format!("Layer-tap: tap {}, hold layout {layer}", key_name(tap)),
        KeyAction::BtSelect(profile) => format!("Switch to BLE profile {} and its host", bt_profile_name(profile)),
        KeyAction::BtNext => "Switch to the next BLE profile".to_string(),
        KeyAction::BtPrev => "Switch to the previous BLE profile".to_string(),
        KeyAction::BtClear => "Forget the host of the current BLE profile".to_string(),
    }
}

/// How a combo key is matched, e.g. `(2, 4)` or `Esc`; a position is prefixed with the action
/// of that key in `layout`, e.g. `Fn 1/2 (2, 4)`.
pub fn combo_ident_label(key: &ComboKey, layout: Option<&Layout>) -> String {
    match combo_coord_action(key, layout) {
        Some((label, (x, y))) => format!("{label} ({x}, {y})"),
        None => combo_ident_short(key),
    }
}

/// Combo key with its count, e.g. `Fn 1/2 (2, 4) hold` or `Esc ×5`, see [`combo_ident_label`].
pub fn combo_key_label(key: &ComboKey, layout: Option<&Layout>) -> String {
    with_count(combo_ident_label(key, layout), key.count)
}

/// Short combo key for a chip, e.g. `Fn 1/2 hold` or `Esc ×5`, see [`combo_chord_key_chip`].
pub fn combo_key_chip(key: &ComboKey, layout: Option<&Layout>) -> String {
    with_count(combo_chord_key_chip(key, layout), key.count)
}

/// Short combo key without the count, as a chord key is: a position is named by the action of that key
/// in `layout`, the coordinates only if the key is empty.
pub fn combo_chord_key_chip(key: &ComboKey, layout: Option<&Layout>) -> String {
    match combo_coord_action(key, layout) {
        Some((label, _)) => label,
        None => combo_ident_short(key),
    }
}

/// Label of the action at the position of a [`ComboKeyIdent::Coord`] key in `layout`, with the position;
/// `None` for an empty key or a key matched by action.
fn combo_coord_action(key: &ComboKey, layout: Option<&Layout>) -> Option<(String, (u8, u8))> {
    let ComboKeyIdent::Coord { x, y } = key.ident else { return None };
    let action = layout?.get(x as usize, y as usize)?;
    (*action != KeyAction::None).then(|| (action_label(action), (x, y)))
}

/// `(x, y)` of a position, the action label of an action (`None` for [`KeyAction::None`]).
fn combo_ident_short(key: &ComboKey) -> String {
    match key.ident {
        ComboKeyIdent::Coord { x, y } => format!("({x}, {y})"),
        ComboKeyIdent::Action(KeyAction::None) => "None".to_string(),
        ComboKeyIdent::Action(action) => action_label(&action),
    }
}

fn with_count(ident: String, count: u8) -> String {
    match count {
        0 => format!("{ident} hold"),
        count => format!("{ident} ×{count}"),
    }
}

/// Combo action, `layouts` are the layout names.
pub fn combo_action_label(action: &ComboAction, layouts: &[String]) -> String {
    match *action {
        ComboAction::SwitchLayout => "Switch to the next layout".to_string(),
        ComboAction::SetLayout(index) => match layouts.get(index as usize) {
            Some(name) => format!("Set layout {index} ({name})"),
            None => format!("Set layout {index} (missing)"),
        },
        ComboAction::Unpair => "Unpair (forget the host of the current BLE profile)".to_string(),
        ComboAction::ResetConfig => "Reset config to factory".to_string(),
        ComboAction::BtSelect(profile) => format!("Switch to BLE profile {}", bt_profile_name(profile)),
        ComboAction::BtNext => "Switch to the next BLE profile".to_string(),
        ComboAction::BtPrev => "Switch to the previous BLE profile".to_string(),
        ComboAction::BtClear => "Forget the host of the current BLE profile".to_string(),
        ComboAction::BtClearAll => "Forget the hosts of all BLE profiles".to_string(),
        ComboAction::Action(KeyAction::None) => "Chord: nothing".to_string(),
        ComboAction::Action(action) => format!("Chord: {}", action_label(&action)),
    }
}

/// `index: NAME`, for layout selectors.
pub fn layout_choice(index: usize, layouts: &[String]) -> String {
    match layouts.get(index) {
        Some(name) => format!("{index}: {name}"),
        None => format!("{index}: (missing)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_complete_and_ordered() {
        for (i, key) in KEYS.iter().enumerate() {
            assert_eq!(key.usage() as usize, i, "{key:?}");
        }
        assert_eq!(KEYS.last(), Some(&Key::F24));
    }

    #[test]
    fn shift_with_a_symbol_key_shows_the_symbol() {
        assert_eq!(action_label(&KeyAction::KeyWithMod(Key::Num1, KeyMod::LeftShift)), "!");
        assert_eq!(action_label(&KeyAction::KeyWithMod(Key::A, KeyMod::LeftShift)), "Shift+A");
        assert_eq!(action_label(&KeyAction::KeyWithMod(Key::Num1, KeyMod::LeftCtrl)), "Ctrl+1");
    }

    #[test]
    fn key_groups_cover_every_key_once() {
        let mut indices: Vec<usize> =
            KEY_GROUPS.iter().flat_map(|(_, ranges)| ranges.iter().cloned().flatten()).collect();
        indices.sort();
        assert_eq!(indices, (0..KEYS.len()).collect::<Vec<_>>());
    }
}
