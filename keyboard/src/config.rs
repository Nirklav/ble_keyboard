use std::io;

use log::{error, info};
use protocol::ble::{ErrorCode, Response};
use protocol::config::Config;
use protocol::keymap::Keymap;

use crate::config_store;
use crate::layout::default_config;
use crate::sc::SpecialCombinations;
use crate::services::Services;

/// Config change waiting in [`Services::pending_config`] until all keys are up.
pub enum Pending {
    /// Save, then apply.
    Apply(Config),
    /// Delete the saved config, then apply the factory one.
    Reset,
}

/// Where the boot config came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Saved,
    /// Nothing is saved.
    Default,
    /// The saved config is broken.
    Fallback,
    /// The boot key is held, the saved config is ignored but kept.
    Bypass,
}

impl Source {
    /// Text for the display, if the user should know about it.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Source::Saved | Source::Default => None,
            Source::Fallback => Some("Default config"),
            Source::Bypass => Some("Default config (boot)"),
        }
    }
}

/// Config to start with, `bypass` ignores the saved one.
pub fn load(width: u8, height: u8, bypass: bool) -> (Config, Source) {
    if bypass {
        info!("Boot key is held, saved config ignored");
        return (default_config(), Source::Bypass);
    }

    match config_store::load(width, height) {
        Ok(Some(config)) => {
            info!("Saved config loaded");
            (config, Source::Saved)
        },
        Ok(None) => {
            info!("No saved config, using default");
            (default_config(), Source::Default)
        },
        Err(e) => {
            error!("Saved config is broken, using default: {}", e);
            (default_config(), Source::Fallback)
        }
    }
}

/// Active config encoded with postcard.
pub fn current_bytes<const W: usize, const H: usize>(svc: &Services<W, H>) -> Vec<u8> {
    svc.config.to_bytes()
}

/// Applies [`Services::pending_config`], must be called when all keys are up.
/// Returns `None` if nothing was pending, otherwise whether the change reached the flash;
/// the config is applied in either case.
pub fn apply_pending<const W: usize, const H: usize>(
    svc: &mut Services<W, H>,
    special_combinations: &mut SpecialCombinations,
) -> anyhow::Result<Option<io::Result<()>>> {
    let Some(pending) = svc.pending_config.take() else {
        return Ok(None);
    };

    let (config, stored) = match pending {
        Pending::Apply(config) => {
            let stored = config_store::save(&config);
            (config, stored)
        },
        Pending::Reset => (default_config(), config_store::remove_saved()),
    };

    apply(config, svc, special_combinations);

    if let Err(e) = &stored {
        error!("Failed to store config: {}", e);
        svc.display.show_text("Failed")?;
    }
    Ok(Some(stored))
}

/// Applies [`Services::pending_config`], then a change submitted over BLE and answers it.
/// Must be called when all keys are up.
pub fn apply_changes<const W: usize, const H: usize>(
    svc: &mut Services<W, H>,
    special_combinations: &mut SpecialCombinations,
) -> anyhow::Result<()> {
    if apply_pending(svc, special_combinations)?.is_some() {
        svc.keyboard.config_service().set_current(current_bytes(svc));
    }

    let Some(pending) = svc.keyboard.config_service().take_submitted() else {
        return Ok(());
    };
    let message = match pending {
        Pending::Apply(_) => "Updated",
        Pending::Reset => "Reset",
    };

    svc.pending_config = Some(pending);
    let stored = apply_pending(svc, special_combinations);
    svc.keyboard.config_service().set_current(current_bytes(svc));

    let response = match &stored {
        Ok(Some(Ok(()))) => Response::Ok,
        // The config is applied, but it's lost on reboot.
        _ => Response::Error(ErrorCode::Storage),
    };
    let ok = response == Response::Ok;
    svc.keyboard.config_service().finish(response);

    stored?;
    if ok {
        svc.display.show_text(message)?;
    }
    Ok(())
}

fn apply<const W: usize, const H: usize>(config: Config, svc: &mut Services<W, H>, special_combinations: &mut SpecialCombinations) {
    info!("Applying config");
    svc.keymap = Keymap::new(&config);
    special_combinations.set(&config.combos);
    svc.config = config;
}
