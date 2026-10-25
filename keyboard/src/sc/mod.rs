use std::fmt::Write;

use log::{error, info};
use protocol::config::{recovery_combo, Combo, ComboAction, ComboKey, ComboKeyIdent};

use crate::config::Pending;
use crate::key_matrix::KeyChange;
use crate::services::Services;

#[derive(Debug)]
struct ScAction {
    action: ComboAction,
    buf: String,
    /// Presses left that the display shows, so it's redrawn only when they change.
    shown_left: Option<u8>
}

impl ScAction {
    fn new(action: ComboAction) -> Self {
        Self {
            action,
            buf: String::new(),
            shown_left: None
        }
    }

    fn on_progress<const W: usize, const H: usize>(&mut self, combination: &[ScKey], svc: &mut Services<W, H>) -> anyhow::Result<()> {
        let label = match self.action {
            ComboAction::Unpair => "Unpair",
            ComboAction::ResetConfig => "Reset",
            ComboAction::SwitchLayout | ComboAction::SetLayout(_) => return Ok(())
        };

        let left = combination.iter()
            .filter(|sc_key| sc_key.required_count > 0 && sc_key.count > 0)
            .map(|sc_key| sc_key.required_count.saturating_sub(sc_key.count))
            .last();
        if left.is_none() || left == self.shown_left {
            return Ok(());
        }
        self.shown_left = left;

        self.buf.clear();
        let _ = write!(self.buf, "{}: {}", label, left.unwrap_or_default());
        svc.display.show_text(&self.buf)?;
        Ok(())
    }

    fn execute<const W: usize, const H: usize>(&mut self, svc: &mut Services<W, H>) -> anyhow::Result<()> {
        match self.action {
            ComboAction::SwitchLayout => {
                let new_layout = svc.key_matrix.switch_layout();
                svc.display.show_text(new_layout.name())?;
            },
            ComboAction::SetLayout(idx) => {
                match svc.key_matrix.set_layout(idx as usize) {
                    Some(new_layout) => svc.display.show_text(new_layout.name())?,
                    None => error!("SetLayout({}) is out of range, ignored.", idx)
                }
            },
            ComboAction::Unpair => {
                svc.display.show_text("Unpaired")?;
                svc.keyboard.unpair();
            },
            ComboAction::ResetConfig => {
                info!("Config reset requested");
                svc.pending_config = Some(Pending::Reset);
                svc.display.show_text("Reset")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct SpecialCombination {
    action: ScAction,
    combination: Vec<ScKey>,
    invalid: bool
}

impl SpecialCombination {
    pub fn from_config(combo: &Combo) -> Self {
        Self {
            action: ScAction::new(combo.action),
            combination: combo.keys.iter().map(ScKey::new).collect(),
            invalid: false
        }
    }

    /// Presses count only while the held keys of the combination are held,
    /// any other matrix key or releasing a held key breaks the combination.
    /// A joystick button that isn't part of the combination breaks it when released, holding it is ignored.
    pub fn changed(&mut self, key: &KeyChange) {
        let Some(action) = key.reg_as else {
            return;
        };

        let holding = self.holding();
        let mut any_matched = false;

        for sc_key in &mut self.combination {
            let matched = match &sc_key.ident {
                ComboKeyIdent::Coord { x, y } => key.coord == Some((*x, *y)),
                ComboKeyIdent::Action(ka) => action == *ka,
            };
            if !matched {
                continue;
            }
            any_matched = true;

            if key.state {
                sc_key.pressed = true;
            } else if sc_key.pressed {
                sc_key.pressed = false;
                if sc_key.required_count == 0 || !holding {
                    self.invalid = true;
                } else {
                    sc_key.count = sc_key.count.saturating_add(1);
                }
            }
        }

        // A joystick button breaks it only once clicked, while held it may be a drag.
        if !any_matched && (key.coord.is_some() || !key.state) {
            self.invalid = true;
        }
    }

    fn holding(&self) -> bool {
        self.combination.iter().all(|sc_key| sc_key.required_count > 0 || sc_key.pressed)
    }

    pub fn try_apply<const W: usize, const H: usize>(&mut self, svc: &mut Services<W, H>) -> anyhow::Result<()> {
        if self.invalid {
            self.reset();
            return Ok(());
        }

        let matched = self.holding() && self.combination.iter()
            .all(|sc_key| sc_key.count >= sc_key.required_count);

        self.action.on_progress(&self.combination, svc)?;

        if matched {
            self.reset();
            self.action.execute(svc)?;
        }

        Ok(())
    }

    pub fn reset(&mut self) {
        for sc_key in &mut self.combination {
            sc_key.pressed = false;
            sc_key.count = 0;
        }

        self.invalid = false;
        self.action.shown_left = None;
    }
}

/// Runs every combination against the same key changes.
/// The recovery combination always follows the configured ones.
pub struct SpecialCombinations {
    combinations: Vec<SpecialCombination>
}

impl SpecialCombinations {
    pub fn from_config(combos: &[Combo]) -> Self {
        let combinations = combos.iter()
            .chain(std::iter::once(&recovery_combo()))
            .map(SpecialCombination::from_config)
            .collect();

        Self {
            combinations
        }
    }

    /// All progress is lost.
    pub fn set(&mut self, combos: &[Combo]) {
        *self = Self::from_config(combos);
    }

    pub fn changed(&mut self, key: &KeyChange) {
        for sc in &mut self.combinations {
            sc.changed(key);
        }
    }

    pub fn try_apply<const W: usize, const H: usize>(&mut self, svc: &mut Services<W, H>) -> anyhow::Result<()> {
        for sc in &mut self.combinations {
            sc.try_apply(svc)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        for sc in &mut self.combinations {
            sc.reset();
        }
    }
}

#[derive(Debug)]
struct ScKey {
    ident: ComboKeyIdent,
    pressed: bool,
    required_count: u8,
    count: u8
}

impl ScKey {
    fn new(key: &ComboKey) -> Self {
        Self {
            ident: key.ident,
            pressed: false,
            required_count: key.count,
            count: 0
        }
    }
}
