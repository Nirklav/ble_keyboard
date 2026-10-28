//! Config edits that keep the layout indices consistent, independent of the UI.

use desktop::config_file;
use protocol::hid::{self, Info};
use protocol::config::{
    Combo, ComboAction, ComboKey, ComboKeyIdent, Config, JoystickButton, JoystickButtons, KeyAction, Layout,
    MAX_COMBO_KEYS, MAX_COMBOS, MAX_LAYOUT_NAME_LEN, MAX_LAYOUTS,
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
    if size > hid::MAX_CONFIG_SIZE {
        return Err(format!("encoded config is {size} bytes, max is {}", hid::MAX_CONFIG_SIZE));
    }
    Ok(())
}

/// Kind of [`KeyAction`], what the action editor switches between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Key,
    KeyWithMod,
    Mod,
    ModTap,
    Media,
    MouseKey,
    MouseTravel,
    MouseScroll,
    Fn,
    Momentary,
    Toggle,
    To,
    LayerTap,
    BtSelect,
    BtNext,
    BtPrev,
    BtClear,
    Transparent,
    None,
}

impl ActionKind {
    /// In the order the editor offers them: keys, mouse, layers, BLE profiles, nothing.
    pub const ALL: [ActionKind; 19] = [
        ActionKind::Key,
        ActionKind::KeyWithMod,
        ActionKind::Mod,
        ActionKind::ModTap,
        ActionKind::Media,
        ActionKind::MouseKey,
        ActionKind::MouseTravel,
        ActionKind::MouseScroll,
        ActionKind::Fn,
        ActionKind::Momentary,
        ActionKind::Toggle,
        ActionKind::To,
        ActionKind::LayerTap,
        ActionKind::BtSelect,
        ActionKind::BtNext,
        ActionKind::BtPrev,
        ActionKind::BtClear,
        ActionKind::Transparent,
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
            KeyAction::Transparent => ActionKind::Transparent,
            KeyAction::Momentary(_) => ActionKind::Momentary,
            KeyAction::Toggle(_) => ActionKind::Toggle,
            KeyAction::To(_) => ActionKind::To,
            KeyAction::ModTap { .. } => ActionKind::ModTap,
            KeyAction::LayerTap { .. } => ActionKind::LayerTap,
            KeyAction::BtSelect(_) => ActionKind::BtSelect,
            KeyAction::BtNext => ActionKind::BtNext,
            KeyAction::BtPrev => ActionKind::BtPrev,
            KeyAction::BtClear => ActionKind::BtClear,
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
            ActionKind::Momentary => "Layer while held (MO)",
            ActionKind::Toggle => "Toggle layer (TG)",
            ActionKind::To => "Switch to layer (TO)",
            ActionKind::ModTap => "Mod-tap: key / modifiers",
            ActionKind::LayerTap => "Layer-tap: key / layer (LT)",
            ActionKind::BtSelect => "BLE profile: select",
            ActionKind::BtNext => "BLE profile: next",
            ActionKind::BtPrev => "BLE profile: previous",
            ActionKind::BtClear => "BLE profile: forget host",
            ActionKind::Transparent => "Transparent",
            ActionKind::None => "None",
        }
    }

    /// An action of this kind made from `from`, keeping its key, modifiers and layout where possible.
    pub fn convert(self, from: &KeyAction) -> KeyAction {
        let key = match *from {
            KeyAction::Key(key)
            | KeyAction::KeyWithMod(key, _)
            | KeyAction::ModTap { tap: key, .. }
            | KeyAction::LayerTap { tap: key, .. } => key,
            _ => Key::A,
        };
        let mods = match *from {
            KeyAction::KeyWithMod(_, mods) | KeyAction::Mod(mods) | KeyAction::ModTap { mods, .. }
                if !mods.is_empty() => mods,
            _ => KeyMod::LeftShift,
        };
        let layer = from.layers().next().unwrap_or(0);
        match self {
            ActionKind::Key => KeyAction::Key(key),
            ActionKind::KeyWithMod => KeyAction::KeyWithMod(key, mods),
            ActionKind::Mod => KeyAction::Mod(mods),
            ActionKind::Media => KeyAction::Media(MediaKey::PlayPause),
            ActionKind::MouseKey => KeyAction::MouseKey(MouseKey::Left),
            ActionKind::MouseTravel => KeyAction::MouseTravel { dx: 0, dy: -10 },
            ActionKind::MouseScroll => KeyAction::MouseScroll { dy: 10 },
            ActionKind::Fn => KeyAction::Fn { temporary: layer, permanent: layer },
            ActionKind::Momentary => KeyAction::Momentary(layer),
            ActionKind::Toggle => KeyAction::Toggle(layer),
            ActionKind::To => KeyAction::To(layer),
            ActionKind::ModTap => KeyAction::ModTap { mods, tap: key },
            ActionKind::LayerTap => KeyAction::LayerTap { layer, tap: key },
            ActionKind::BtSelect => KeyAction::BtSelect(from.bt_profile().unwrap_or(0)),
            ActionKind::BtNext => KeyAction::BtNext,
            ActionKind::BtPrev => KeyAction::BtPrev,
            ActionKind::BtClear => KeyAction::BtClear,
            ActionKind::Transparent => KeyAction::Transparent,
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
pub fn add_layout(config: &mut Config, layout: Layout) -> Result<usize, String> {
    if config.layouts.len() >= MAX_LAYOUTS {
        return Err(format!("at most {MAX_LAYOUTS} layouts are supported"));
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

/// What points to layout `index`: layer keys and joystick buttons (Fn, MO, TG, TO, LT), `SetLayout` combos
/// and combo keys matching such layer keys.
pub fn layout_references(config: &Config, index: usize) -> Vec<String> {
    let refers = |action: &KeyAction| action.layers().any(|layer| layer as usize == index);
    let kind = |action: &KeyAction| if matches!(action, KeyAction::Fn { .. }) { "Fn" } else { "Layer" };

    let mut references = Vec::new();
    for (l, layout) in config.layouts.iter().enumerate() {
        if l == index {
            // Layer keys of the removed layout go away with it.
            continue;
        }
        for (y, row) in layout.rows.iter().enumerate() {
            for (x, action) in row.iter().enumerate() {
                if refers(action) {
                    references.push(format!("{} key ({x}, {y}) of layout {l} \"{}\"", kind(action), layout.name));
                }
            }
        }
        for button in JoystickButton::ALL {
            if refers(layout.joystick.get(button)) {
                let button = button.name().to_lowercase();
                references.push(format!("joystick {button} button of layout {l} \"{}\"", layout.name));
            }
        }
    }
    for (c, combo) in config.combos.iter().enumerate() {
        if combo.action == ComboAction::SetLayout(index as u8) {
            references.push(format!("combo {c}: SetLayout({index})"));
        }
        if combo.keys.iter().any(|k| matches!(&k.ident, ComboKeyIdent::Action(action) if refers(action))) {
            references.push(format!("combo {c}: key matching a layer key"));
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

/// Keeps the layer keys and combos pointing to the same layouts.
pub fn swap_layouts(config: &mut Config, a: usize, b: usize) {
    config.layouts.swap(a, b);
    let (a, b) = (a as u8, b as u8);
    remap_layout_indices(config, |i| match i {
        i if i == a => b,
        i if i == b => a,
        i => i,
    });
}

/// Applies `map` to every layout index: layer keys and joystick buttons, `SetLayout` combos,
/// combo keys matching layer keys.
fn remap_layout_indices(config: &mut Config, map: impl Fn(u8) -> u8) {
    let remap = |action: &mut KeyAction| action.map_layers(&map);
    for layout in &mut config.layouts {
        for action in layout.rows.iter_mut().flatten() {
            remap(action);
        }
        for button in JoystickButton::ALL {
            remap(layout.joystick.get_mut(button));
        }
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
        let from = KeyAction::ModTap { mods: KeyMod::LeftAlt, tap: Key::F };
        assert_eq!(ActionKind::KeyWithMod.convert(&from), KeyAction::KeyWithMod(Key::F, KeyMod::LeftAlt));
        assert_eq!(ActionKind::LayerTap.convert(&from), KeyAction::LayerTap { layer: 0, tap: Key::F });
        assert_eq!(ActionKind::Toggle.convert(&KeyAction::Momentary(2)), KeyAction::Toggle(2));
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
    fn set_layout_combo_and_combo_key_are_references() {
        let mut config = example();
        config.combos[0].action = ComboAction::SetLayout(2);
        config.combos[1].keys[0].ident = ComboKeyIdent::Action(KeyAction::Fn { temporary: 1, permanent: 2 });
        let references = layout_references(&config, 2);
        assert!(references.contains(&"combo 0: SetLayout(2)".to_string()), "{references:?}");
        assert!(references.contains(&"combo 1: key matching a layer key".to_string()), "{references:?}");
    }

    #[test]
    fn layer_actions_are_references_and_remapped() {
        let mut config = example();
        config.layouts[1].rows[0][0] = KeyAction::Momentary(2);
        config.layouts[0].rows[0][0] = KeyAction::LayerTap { layer: 2, tap: Key::Escape };
        config.layouts[0].joystick.stick = KeyAction::Toggle(2);
        config.combos[0].action = ComboAction::SetLayout(2);
        let references = layout_references(&config, 2);
        assert!(references.contains(&"Layer key (0, 0) of layout 1 \"FUNCTION\"".to_string()), "{references:?}");
        assert!(references.contains(&"joystick stick button of layout 0 \"MAIN\"".to_string()), "{references:?}");

        swap_layouts(&mut config, 1, 2);
        assert_eq!(layout_names(&config), ["MAIN", "GAME", "FUNCTION"]);
        assert_eq!(config.layouts[0].rows[4][2], KeyAction::Fn { temporary: 2, permanent: 1 });
        assert_eq!(config.combos[0].action, ComboAction::SetLayout(1));
        assert_eq!(config.layouts[2].rows[0][0], KeyAction::Momentary(1));
        assert_eq!(config.layouts[0].rows[0][0], KeyAction::LayerTap { layer: 1, tap: Key::Escape });
        assert_eq!(config.layouts[0].joystick.stick, KeyAction::Toggle(1));
        config.validate_for(10, 5).unwrap();
    }

    #[test]
    fn duplicate_layout_name_fits() {
        let mut config = example();
        config.layouts[1].name = "ABCDEFGHIJKL".to_string();
        let index = duplicate_layout(&mut config, 1).unwrap();
        assert_eq!(config.layouts[index].name, "ABCDEFGHIJK2");
        config.validate_for(10, 5).unwrap();
    }
}
