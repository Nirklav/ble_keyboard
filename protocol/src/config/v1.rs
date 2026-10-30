//! Config of version 1: layouts without the joystick buttons.

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
