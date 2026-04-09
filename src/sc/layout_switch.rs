use std::time::Duration;

use crate::display::Display;
use crate::key_matrix::{KeyChange, KeyMatrix};
use crate::layout::Layouts;
use crate::sc::SpecialCombination;

const SCREEN_TEXT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct LayoutSwitch {
    next_fn_switch: bool,
    only_fn_pressed: Option<bool>,
}

impl LayoutSwitch {
    pub fn new() -> Self {
        Self {
            next_fn_switch: false,
            only_fn_pressed: None,
        }
    }
}

impl SpecialCombination for LayoutSwitch {
    fn new_scan(&mut self) {
        self.only_fn_pressed = None;
    }

    fn changed(&mut self, key: &KeyChange) {
        if let Some(layout_key) = key.reg_as {
            if key.state && !layout_key.is_fn() {
                self.only_fn_pressed = Some(false);
                self.next_fn_switch = false;
            }
            if !key.state && key.changed && layout_key.is_fn() && self.only_fn_pressed.is_none() {
                self.only_fn_pressed = Some(true);
            }
        }
    }

    fn apply<const W: usize, const H: usize, const C: usize>(
        &mut self,
        key_matrix: &mut KeyMatrix<W, H>,
        layouts: &'static Layouts<W, H, C>,
        display: &Display,
    ) -> anyhow::Result<()> {
        if let Some(true) = self.only_fn_pressed {
            if self.next_fn_switch {
                self.next_fn_switch = false;
                let new_layout = key_matrix.switch_layout(layouts);
                display.show_text_for(new_layout.name(), SCREEN_TEXT_TIMEOUT)?;
            } else {
                self.next_fn_switch = true;
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.next_fn_switch = false;
    }
}
