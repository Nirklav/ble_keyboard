//! Config of version 3: [`Settings`] of the tap-hold keys only.

use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use super::{Combo, Layout, CONFIG_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub tapping_term_ms: u16,
    pub require_prior_idle_ms: u16,
}

impl Default for Settings {
    fn default() -> Self {
        let super::Settings { tapping_term_ms, require_prior_idle_ms, .. } = super::Settings::default();
        Self { tapping_term_ms, require_prior_idle_ms }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u16,
    pub width: u8,
    pub height: u8,
    pub layouts: Vec<Layout>,
    pub combos: Vec<Combo>,
    #[serde(default)]
    pub settings: Settings,
}

/// The new settings are the default ones.
impl From<Config> for super::Config {
    fn from(config: Config) -> Self {
        let Settings { tapping_term_ms, require_prior_idle_ms } = config.settings;
        Self {
            version: CONFIG_VERSION,
            width: config.width,
            height: config.height,
            layouts: config.layouts,
            combos: config.combos,
            settings: super::Settings { tapping_term_ms, require_prior_idle_ms, ..super::Settings::default() },
        }
    }
}
