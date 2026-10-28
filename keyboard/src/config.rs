use log::{error, info};
use protocol::hid::{ErrorCode, Response};
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

/// Applies [`Services::pending_config`] (a combo's), then a change submitted by the config app and answers it.
/// Must be called when all keys are up.
pub fn apply_changes<const W: usize, const H: usize>(
    svc: &mut Services<W, H>,
    special_combinations: &mut SpecialCombinations,
) -> anyhow::Result<()> {
    if let Some(pending) = svc.pending_config.take() {
        apply_pending(pending, svc, special_combinations)?;
    }

    let Some(pending) = svc.keyboard.config_service().take_submitted() else {
        return Ok(());
    };
    let message = match pending {
        Pending::Apply(_) => "Updated",
        Pending::Reset => "Reset",
    };
    let stored = apply_pending(pending, svc, special_combinations);
    // The config is applied either way, but it's lost on reboot unless stored.
    let response = if matches!(stored, Ok(true)) { Response::Ok } else { Response::Error(ErrorCode::Storage) };
    svc.keyboard.config_service().finish(response);

    if stored? {
        svc.display.show_text(message)?;
    }
    Ok(())
}

/// Applies `pending` and returns whether the change reached the flash; the config is applied in either case.
fn apply_pending<const W: usize, const H: usize>(
    pending: Pending,
    svc: &mut Services<W, H>,
    special_combinations: &mut SpecialCombinations,
) -> anyhow::Result<bool> {
    let (config, stored) = match pending {
        Pending::Apply(config) => {
            let stored = config_store::save(&config);
            (config, stored)
        },
        Pending::Reset => (default_config(), config_store::remove_saved()),
    };

    apply(config, svc, special_combinations);
    svc.keyboard.config_service().set_current(svc.config.to_bytes());

    if let Err(e) = &stored {
        error!("Failed to store config: {}", e);
        svc.display.show_text("Failed")?;
    }
    Ok(stored.is_ok())
}

fn apply<const W: usize, const H: usize>(config: Config, svc: &mut Services<W, H>, special_combinations: &mut SpecialCombinations) {
    info!("Applying config");
    svc.keymap = Keymap::new(&config);
    svc.key_matrix.set_debounce_ms(config.settings.debounce_ms);
    special_combinations.set(&config.combos);
    svc.config = config;
}
