use alloc::string::ToString;
use alloc::vec;

use super::*;

/// 10x5 config with 3 layouts, special combos and a chord, close to the firmware default.
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
            Combo {
                keys: vec![
                    ComboKey { ident: ComboKeyIdent::Coord { x: 5, y: 1 }, count: 0 },
                    ComboKey { ident: ComboKeyIdent::Action(KeyAction::Key(Key::A)), count: 0 },
                ],
                action: ComboAction::Action(KeyAction::KeyWithMod(Key::Z, KeyMod::LeftCtrl)),
            },
        ],
        settings: Settings {
            tapping_term_ms: 180,
            require_prior_idle_ms: 0,
            combo_term_ms: 40,
            debounce_ms: 8,
            joystick_invert_y: true,
            ..Settings::default()
        },
    }
}

#[test]
fn postcard_roundtrip() {
    let config = sample();
    let bytes = config.to_bytes();
    assert_eq!(Config::from_bytes(&bytes).unwrap(), config);
    // Settings may end early (see `settings_are_appended`), what's before them may not.
    let settings = postcard::to_allocvec(&config.settings).unwrap().len();
    assert!(Config::from_bytes(&bytes[..bytes.len() - settings - 1]).is_err());
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
    combos.retain(|combo| !matches!(combo.action, ComboAction::BtSelect(_) | ComboAction::Action(_)));
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
fn version_3_is_upgraded() {
    let config = sample();
    let old = v3::Config {
        version: 3,
        width: config.width,
        height: config.height,
        layouts: config.layouts.clone(),
        combos: config.combos.clone(),
        settings: v3::Settings { tapping_term_ms: 180, require_prior_idle_ms: 0 },
    };
    let upgraded = Config::from_bytes(&postcard::to_allocvec(&old).unwrap()).unwrap();
    assert_eq!(upgraded.version, CONFIG_VERSION);
    assert_eq!(upgraded.settings, Settings { tapping_term_ms: 180, require_prior_idle_ms: 0, ..Settings::default() });
    assert_eq!(upgraded.layouts, config.layouts);
    assert_eq!(upgraded.combos, config.combos);
    assert_eq!(upgraded.validate_for(10, 5), Ok(()));
}

#[test]
fn settings_are_appended() {
    let mut config = sample();
    // All different from the defaults and from each other: read in the wrong order, they would differ.
    config.settings = Settings {
        tapping_term_ms: 210,
        require_prior_idle_ms: 140,
        combo_term_ms: 60,
        debounce_ms: 7,
        sleep_connected_s: 610,
        sleep_disconnected_s: 70,
        joystick_speed: 1900,
        joystick_ramp_ms: 30,
        joystick_invert_x: true,
        joystick_invert_y: true,
        display_off_s: 130,
    };
    let bytes = config.to_bytes();
    assert_eq!(Config::from_bytes(&bytes).unwrap(), config);

    // An older app or firmware has no `display_off_s`: it reads the rest of a newer config...
    type Older = (u16, u8, u8, Vec<Layout>, Vec<Combo>, [u16; 8], bool, bool);
    let older: Older = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(older.5[7], 30);
    assert!(older.7);
    // ...and writes its config without it: the default here.
    let older_bytes = postcard::to_allocvec(&older).unwrap();
    assert!(older_bytes.len() < bytes.len());
    let upgraded = Config::from_bytes(&older_bytes).unwrap();
    assert_eq!(upgraded.settings, Settings { display_off_s: Settings::default().display_off_s, ..config.settings });
    assert_eq!(upgraded.layouts, config.layouts);
    assert_eq!(upgraded.combos, config.combos);
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
    assert_eq!(postcard::to_allocvec(&ComboAction::Action(KeyAction::Key(Key::A))).unwrap(), [9, 0, 4]);
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
        version = 4
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

        [[combos]]
        keys = [{ ident = { Coord = { x = 0, y = 0 } }, count = 0 }, { ident = { Coord = { x = 1, y = 0 } }, count = 0 }]
        action = { Action = { Key = "Escape" } }
    "#;
    let config: Config = toml::from_str(text).unwrap();
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.settings, Settings { tapping_term_ms: 250, ..Settings::default() });
    assert_eq!(config.combos[3].action, ComboAction::Action(KeyAction::Key(Key::Escape)));
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
    config.validate().unwrap_err()
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
        invalid(|c| c.combos[0].action = ComboAction::SetLayout(3)),
        ValidationError::ComboLayoutIndex { combo: 0, index: 3 }
    );
    assert_eq!(
        invalid(|c| c.layouts[1].rows[2][3] = KeyAction::BtSelect(BT_PROFILES as u8)),
        ValidationError::Key { layout: 1, x: 3, y: 2, error: ActionError::BtProfileIndex(BT_PROFILES as u8) }
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
        invalid(|c| c.settings.combo_term_ms = 5),
        ValidationError::Setting { name: "combo_term_ms", value: 5, min: 10, max: 200 }
    );
    assert_eq!(
        invalid(|c| c.settings.sleep_disconnected_s = 10),
        ValidationError::Setting { name: "sleep_disconnected_s", value: 10, min: 15, max: 36000 }
    );
    assert_eq!(
        invalid(|c| c.settings.display_off_s = 5),
        ValidationError::Setting { name: "display_off_s", value: 5, min: 10, max: 3600 }
    );
    assert_eq!(
        invalid(|c| c.settings.joystick_ramp_ms = 600),
        ValidationError::Setting { name: "joystick_ramp_ms", value: 600, min: 0, max: 500 }
    );
    assert_eq!(invalid(|c| { c.combos[4].keys.pop(); }), ValidationError::ChordTooFewKeys { combo: 4 });
    assert_eq!(invalid(|c| c.combos[4].keys[1].count = 2), ValidationError::ChordKeyCount { combo: 4 });
    assert_eq!(
        invalid(|c| c.combos[4].keys[1].ident = ComboKeyIdent::Coord { x: 5, y: 1 }),
        ValidationError::ChordDuplicateKey { combo: 4 }
    );
    let tap_hold = KeyAction::LayerTap { layer: 1, tap: Key::A };
    assert_eq!(invalid(|c| c.combos[4].action = ComboAction::Action(tap_hold)), ValidationError::ChordActionKind { combo: 4 });
    assert_eq!(
        invalid(|c| c.combos[4].action = ComboAction::Action(KeyAction::Momentary(3))),
        ValidationError::ChordAction { combo: 4, error: ActionError::LayerIndex(3) }
    );
    let no_mods = KeyAction::ModTap { mods: KeyMod::empty(), tap: Key::A };
    assert_eq!(
        invalid(|c| c.layouts[1].rows[2][3] = no_mods),
        ValidationError::Key { layout: 1, x: 3, y: 2, error: ActionError::ModTapWithoutMods }
    );
    assert_eq!(
        sample().validate_for(12, 5),
        Err(ValidationError::WrongKeyboardSize { expected_width: 12, expected_height: 5, width: 10, height: 5 })
    );
}
