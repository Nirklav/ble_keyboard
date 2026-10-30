//! Config of version 2: no [`Settings`].

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
