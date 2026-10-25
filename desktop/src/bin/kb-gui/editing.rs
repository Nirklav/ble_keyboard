//! Config edits that keep the layout indices consistent, independent of the UI.

use desktop::config_file;
use protocol::ble::{self, Info};
use protocol::config::{
    Combo, ComboAction, ComboKey, ComboKeyIdent, Config, JoystickButtons, KeyAction, Layout, MAX_COMBO_KEYS,
    MAX_COMBOS, MAX_LAYOUT_NAME_LEN, MAX_LAYOUTS,
};
use protocol::keys::{Key, KeyMod, MediaKey, MouseKey};

/// Keyboard size used for validation while no keyboard is connected.
pub const DEFAULT_SIZE: (u8, u8) = (10, 5);

/// Checks what blocks a push: validity for the keyboard size, the keyboard limits, the encoded size.
/// `info` is the connected keyboard, if any.
pub fn check(config: &Config, info: Option<&Info>) -> Result<(), String> {
    let (width, height) = info.map_or(DEFAULT_SIZE, |info| (info.width, info.height));
    config.validate_for(width, height).map_err(|e| config_file::describe(&e, config))?;
    if let Some(info) = info {
        if config.layouts.len() > info.max_layouts as usize {
            return Err(format!("the keyboard supports at most {} layouts", info.max_layouts));
        }
        if config.combos.len() > info.max_combos as usize {
            return Err(format!("the keyboard supports at most {} combos", info.max_combos));
        }
    }
    let size = config.to_bytes().len();
    if size > ble::MAX_CONFIG_SIZE {
        return Err(format!("encoded config is {size} bytes, max is {}", ble::MAX_CONFIG_SIZE));
    }
    Ok(())
}

/// Kind of [`KeyAction`], what the action editor switches between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Key,
    KeyWithMod,
    Mod,
    Media,
    MouseKey,
    MouseTravel,
    MouseScroll,
    Fn,
    None,
}

impl ActionKind {
    pub const ALL: [ActionKind; 9] = [
        ActionKind::Key,
        ActionKind::KeyWithMod,
        ActionKind::Mod,
        ActionKind::Media,
        ActionKind::MouseKey,
        ActionKind::MouseTravel,
        ActionKind::MouseScroll,
        ActionKind::Fn,
        ActionKind::None,
    ];

    pub fn of(action: &KeyAction) -> ActionKind {
        match action {
            KeyAction::Key(_) => ActionKind::Key,
            KeyAction::KeyWithMod(..) => ActionKind::KeyWithMod,
            KeyAction::Mod(_) => ActionKind::Mod,
            KeyAction::Media(_) => ActionKind::Media,
            KeyAction::MouseKey(_) => ActionKind::MouseKey,
            KeyAction::MouseTravel { .. } => ActionKind::MouseTravel,
            KeyAction::MouseScroll { .. } => ActionKind::MouseScroll,
            KeyAction::Fn { .. } => ActionKind::Fn,
            KeyAction::None => ActionKind::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ActionKind::Key => "Key",
            ActionKind::KeyWithMod => "Key + modifiers",
            ActionKind::Mod => "Modifiers",
            ActionKind::Media => "Media",
            ActionKind::MouseKey => "Mouse button",
            ActionKind::MouseTravel => "Mouse travel",
            ActionKind::MouseScroll => "Mouse scroll",
            ActionKind::Fn => "Fn (layer key)",
            ActionKind::None => "None",
        }
    }

    /// An action of this kind made from `from`, keeping its key and modifiers where possible.
    pub fn convert(self, from: &KeyAction) -> KeyAction {
        let key = match *from {
            KeyAction::Key(key) | KeyAction::KeyWithMod(key, _) => key,
            _ => Key::A,
        };
        let mods = match *from {
            KeyAction::KeyWithMod(_, mods) | KeyAction::Mod(mods) if !mods.is_empty() => mods,
            _ => KeyMod::LeftShift,
        };
        match self {
            ActionKind::Key => KeyAction::Key(key),
            ActionKind::KeyWithMod => KeyAction::KeyWithMod(key, mods),
            ActionKind::Mod => KeyAction::Mod(mods),
            ActionKind::Media => KeyAction::Media(MediaKey::PlayPause),
            ActionKind::MouseKey => KeyAction::MouseKey(MouseKey::Left),
            ActionKind::MouseTravel => KeyAction::MouseTravel { dx: 0, dy: -10 },
            ActionKind::MouseScroll => KeyAction::MouseScroll { dy: 10 },
            ActionKind::Fn => KeyAction::Fn { temporary: 0, permanent: 0 },
            ActionKind::None => KeyAction::None,
        }
    }
}

pub fn layout_names(config: &Config) -> Vec<String> {
    config.layouts.iter().map(|l| l.name.clone()).collect()
}

/// Layout of `config`'s size with every key set to [`KeyAction::None`];
/// the joystick buttons stay mouse buttons, so they work in a new `Fn` layer.
pub fn empty_layout(config: &Config, name: String) -> Layout {
    Layout {
        name,
        rows: vec![vec![KeyAction::None; config.width as usize]; config.height as usize],
        joystick: JoystickButtons::default(),
    }
}

/// `LAYER<n>` that no layout has yet.
pub fn unique_layout_name(config: &Config) -> String {
    (config.layouts.len() + 1..)
        .map(|n| format!("LAYER{n}"))
        .find(|name| config.layouts.iter().all(|l| &l.name != name))
        .expect("there is always a free name")
}

/// Appends `layout`, returns its index.
pub fn add_layout(config: &mut Config, mut layout: Layout) -> Result<usize, String> {
    if config.layouts.len() >= MAX_LAYOUTS {
        return Err(format!("at most {MAX_LAYOUTS} layouts are supported"));
    }
    if layout.name.chars().count() > MAX_LAYOUT_NAME_LEN {
        layout.name = layout.name.chars().take(MAX_LAYOUT_NAME_LEN).collect();
    }
    config.layouts.push(layout);
    Ok(config.layouts.len() - 1)
}

/// Appends a copy of layout `index` with a new name, returns its index.
pub fn duplicate_layout(config: &mut Config, index: usize) -> Result<usize, String> {
    let mut layout = config.layouts[index].clone();
    let suffix = "2";
    let base: String = layout.name.chars().take(MAX_LAYOUT_NAME_LEN - suffix.len()).collect();
    layout.name = format!("{base}{suffix}");
    add_layout(config, layout)
}

/// What points to layout `index`: Fn keys, `SetLayout` combos and combo keys matching such Fn keys.
pub fn layout_references(config: &Config, index: usize) -> Vec<String> {
    let refers = |action: &KeyAction| {
        matches!(*action, KeyAction::Fn { temporary, permanent }
            if temporary as usize == index || permanent as usize == index)
    };

    let mut references = Vec::new();
    for (l, layout) in config.layouts.iter().enumerate() {
        if l == index {
            // Fn keys of the removed layout go away with it.
            continue;
        }
        for (y, row) in layout.rows.iter().enumerate() {
            for (x, action) in row.iter().enumerate() {
                if refers(action) {
                    references.push(format!("Fn key ({x}, {y}) of layout {l} \"{}\"", layout.name));
                }
            }
        }
    }
    for (c, combo) in config.combos.iter().enumerate() {
        if combo.action == ComboAction::SetLayout(index as u8) {
            references.push(format!("combo {c}: SetLayout({index})"));
        }
        if combo.keys.iter().any(|k| matches!(&k.ident, ComboKeyIdent::Action(action) if refers(action))) {
            references.push(format!("combo {c}: key matching an Fn key"));
        }
    }
    references
}

/// Shifts the indices of the following layouts.
/// Refuses to remove the last layout or a layout something points to.
pub fn remove_layout(config: &mut Config, index: usize) -> Result<(), String> {
    if config.layouts.len() <= 1 {
        return Err("the config needs at least one layout".to_string());
    }
    let references = layout_references(config, index);
    if !references.is_empty() {
        return Err(format!(
            "layout {index} \"{}\" is used by:\n{}\nChange them first.",
            config.layouts[index].name,
            references.join("\n")
        ));
    }

    config.layouts.remove(index);
    let index = index as u8;
    remap_layout_indices(config, |i| if i > index { i - 1 } else { i });
    Ok(())
}

/// Keeps the Fn keys and combos pointing to the same layouts.
pub fn swap_layouts(config: &mut Config, a: usize, b: usize) {
    config.layouts.swap(a, b);
    let (a, b) = (a as u8, b as u8);
    remap_layout_indices(config, |i| match i {
        i if i == a => b,
        i if i == b => a,
        i => i,
    });
}

/// Applies `map` to every layout index: Fn keys, `SetLayout` combos, combo keys matching Fn keys.
fn remap_layout_indices(config: &mut Config, map: impl Fn(u8) -> u8) {
    let remap = |action: &mut KeyAction| {
        if let KeyAction::Fn { temporary, permanent } = action {
            *temporary = map(*temporary);
            *permanent = map(*permanent);
        }
    };
    for action in config.layouts.iter_mut().flat_map(|l| l.rows.iter_mut()).flat_map(|r| r.iter_mut()) {
        remap(action);
    }
    for combo in &mut config.combos {
        if let ComboAction::SetLayout(index) = &mut combo.action {
            *index = map(*index);
        }
        for key in &mut combo.keys {
            if let ComboKeyIdent::Action(action) = &mut key.ident {
                remap(action);
            }
        }
    }
}

/// Coordinate of another Fn key in `layout` than `(x, y)`, if any: a layout can have only one.
pub fn other_fn_key(layout: &Layout, x: usize, y: usize) -> Option<(usize, usize)> {
    layout.rows.iter().enumerate().find_map(|(fy, row)| {
        row.iter()
            .enumerate()
            .find(|&(fx, action)| (fx, fy) != (x, y) && matches!(action, KeyAction::Fn { .. }))
            .map(|(fx, _)| (fx, fy))
    })
}

/// Button of the joystick, see [`JoystickButtons`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub fn action(self, buttons: &JoystickButtons) -> &KeyAction {
        match self {
            JoystickButton::Left => &buttons.left,
            JoystickButton::Stick => &buttons.stick,
            JoystickButton::Right => &buttons.right,
        }
    }

    pub fn action_mut(self, buttons: &mut JoystickButtons) -> &mut KeyAction {
        match self {
            JoystickButton::Left => &mut buttons.left,
            JoystickButton::Stick => &mut buttons.stick,
            JoystickButton::Right => &mut buttons.right,
        }
    }
}

/// New combo: double tap the key at (0, 0) to switch the layout.
pub fn new_combo() -> Combo {
    Combo { keys: vec![ComboKey { count: 2, ..new_combo_key() }], action: ComboAction::SwitchLayout }
}

/// New combo key: hold the key at (0, 0).
pub fn new_combo_key() -> ComboKey {
    ComboKey { ident: ComboKeyIdent::Coord { x: 0, y: 0 }, count: 0 }
}

/// Appends a new combo, returns its index.
pub fn add_combo(config: &mut Config) -> Result<usize, String> {
    if config.combos.len() >= MAX_COMBOS {
        return Err(format!("at most {MAX_COMBOS} combos are supported"));
    }
    config.combos.push(new_combo());
    Ok(config.combos.len() - 1)
}

/// Appends a new key to `combo`, returns its index.
pub fn add_combo_key(combo: &mut Combo) -> Result<usize, String> {
    if combo.keys.len() >= MAX_COMBO_KEYS {
        return Err(format!("at most {MAX_COMBO_KEYS} keys per combo are supported"));
    }
    combo.keys.push(new_combo_key());
    Ok(combo.keys.len() - 1)
}

/// Combo keys that point to `(x, y)`, for highlighting the grid.
pub fn combo_uses_coord(combo: &Combo, x: usize, y: usize) -> bool {
    combo
        .keys
        .iter()
        .any(|k| matches!(k.ident, ComboKeyIdent::Coord { x: cx, y: cy } if (cx as usize, cy as usize) == (x, y)))
}

#[cfg(test)]
mod tests {
    use desktop::EXAMPLE_CONFIG;
    use desktop::config_file;

    use super::*;

    fn example() -> Config {
        config_file::parse(EXAMPLE_CONFIG).unwrap()
    }

    #[test]
    fn check_reports_errors() {
        let mut config = example();
        assert_eq!(check(&config, None), Ok(()));
        config.layouts[1].name = "ABCDEFGHIJKLM".to_string();
        assert_eq!(
            check(&config, None),
            Err("layout 1: name has 13 characters, max is 12 (layout \"ABCDEFGHIJKLM\")".into())
        );

        let config = example();
        let info = Info {
            protocol_version: ble::PROTOCOL_VERSION,
            config_version: protocol::config::CONFIG_VERSION,
            width: 10,
            height: 5,
            max_layouts: 2,
            max_combos: 16,
            firmware_version: "test".into(),
        };
        assert_eq!(check(&config, Some(&info)), Err("the keyboard supports at most 2 layouts".into()));
        let info = Info { width: 12, max_layouts: 8, ..info };
        assert!(check(&config, Some(&info)).unwrap_err().contains("this keyboard is 12x5"));
    }

    #[test]
    fn convert_keeps_key_and_mods() {
        let from = KeyAction::KeyWithMod(Key::Num1, KeyMod::LeftCtrl);
        assert_eq!(ActionKind::Key.convert(&from), KeyAction::Key(Key::Num1));
        assert_eq!(ActionKind::Mod.convert(&from), KeyAction::Mod(KeyMod::LeftCtrl));
        assert_eq!(
            ActionKind::KeyWithMod.convert(&KeyAction::Key(Key::B)),
            KeyAction::KeyWithMod(Key::B, KeyMod::LeftShift)
        );
        for kind in ActionKind::ALL {
            assert_eq!(ActionKind::of(&kind.convert(&from)), kind);
        }
    }

    #[test]
    fn referenced_layout_is_not_removed() {
        let mut config = example();
        // MAIN Fn 1/2 points to FUNCTION and GAME, GAME Fn 1/0 points to FUNCTION and MAIN.
        let error = remove_layout(&mut config, 1).unwrap_err();
        assert!(error.contains("Fn key (2, 4) of layout 0 \"MAIN\""), "{error}");
        assert!(error.contains("Fn key (2, 4) of layout 2 \"GAME\""), "{error}");
        assert_eq!(config.layouts.len(), 3);
    }

    #[test]
    fn remove_layout_shifts_indices() {
        let mut config = example();
        config.layouts[0].rows[4][2] = KeyAction::Fn { temporary: 2, permanent: 2 };
        config.layouts[2].rows[4][2] = KeyAction::Fn { temporary: 0, permanent: 0 };
        config.combos[0].action = ComboAction::SetLayout(2);
        assert!(layout_references(&config, 1).is_empty());

        remove_layout(&mut config, 1).unwrap();
        let names = layout_names(&config);
        assert_eq!(names, ["MAIN", "GAME"]);
        assert_eq!(config.layouts[0].rows[4][2], KeyAction::Fn { temporary: 1, permanent: 1 });
        assert_eq!(config.layouts[1].rows[4][2], KeyAction::Fn { temporary: 0, permanent: 0 });
        assert_eq!(config.combos[0].action, ComboAction::SetLayout(1));
        config.validate_for(10, 5).unwrap();
    }

    #[test]
    fn last_layout_is_not_removed() {
        let mut config = example();
        config.layouts.truncate(1);
        config.layouts[0].rows[4][2] = KeyAction::None;
        assert!(remove_layout(&mut config, 0).is_err());
    }

    #[test]
    fn set_layout_combo_and_combo_key_are_references() {
        let mut config = example();
        config.combos[0].action = ComboAction::SetLayout(2);
        config.combos[1].keys[0].ident = ComboKeyIdent::Action(KeyAction::Fn { temporary: 1, permanent: 2 });
        let references = layout_references(&config, 2);
        assert!(references.contains(&"combo 0: SetLayout(2)".to_string()), "{references:?}");
        assert!(references.contains(&"combo 1: key matching an Fn key".to_string()), "{references:?}");
    }

    #[test]
    fn swap_layouts_keeps_targets() {
        let mut config = example();
        config.combos[0].action = ComboAction::SetLayout(2);
        swap_layouts(&mut config, 1, 2);
        assert_eq!(layout_names(&config), ["MAIN", "GAME", "FUNCTION"]);
        assert_eq!(config.layouts[0].rows[4][2], KeyAction::Fn { temporary: 2, permanent: 1 });
        assert_eq!(config.layouts[1].rows[4][2], KeyAction::Fn { temporary: 2, permanent: 0 });
        assert_eq!(config.combos[0].action, ComboAction::SetLayout(1));
        config.validate_for(10, 5).unwrap();
    }

    #[test]
    fn add_layouts_up_to_limit() {
        let mut config = example();
        while config.layouts.len() < MAX_LAYOUTS {
            let name = unique_layout_name(&config);
            let layout = empty_layout(&config, name);
            add_layout(&mut config, layout).unwrap();
        }
        config.validate_for(10, 5).unwrap();
        let name = unique_layout_name(&config);
        let layout = empty_layout(&config, name);
        assert!(add_layout(&mut config, layout).is_err());
    }

    #[test]
    fn duplicate_layout_name_fits() {
        let mut config = example();
        config.layouts[1].name = "ABCDEFGHIJKL".to_string();
        let index = duplicate_layout(&mut config, 1).unwrap();
        assert_eq!(config.layouts[index].name, "ABCDEFGHIJK2");
        config.validate_for(10, 5).unwrap();
    }

    #[test]
    fn other_fn_key_is_found() {
        let config = example();
        assert_eq!(other_fn_key(&config.layouts[0], 0, 0), Some((2, 4)));
        assert_eq!(other_fn_key(&config.layouts[0], 2, 4), None);
    }

    #[test]
    fn combos_up_to_limits() {
        let mut config = example();
        while config.combos.len() < MAX_COMBOS {
            add_combo(&mut config).unwrap();
        }
        assert!(add_combo(&mut config).is_err());

        let combo = &mut config.combos[0];
        while combo.keys.len() < MAX_COMBO_KEYS {
            add_combo_key(combo).unwrap();
        }
        assert!(add_combo_key(combo).is_err());
        assert!(combo_uses_coord(combo, 2, 4));
        config.validate_for(10, 5).unwrap();
    }
}
