use std::fmt::Write;

use log::info;

use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::display::Display;
use crate::key_matrix::{KeyChange, KeyMatrix};
use crate::layout::{LayoutKey, Layouts};

const REQUIRED_PRESSES: u8 = 3;

pub struct Unpair {
    target: (usize, usize),
    press_count: u8,
    target_pressed: bool,
    invalid: bool,
    buf: String,
}

impl Unpair {
    pub fn new(target_x: usize, target_y: usize) -> Self {
        Self {
            target: (target_x, target_y),
            press_count: 0,
            target_pressed: false,
            invalid: false,
            buf: String::new(),
        }
    }

    pub fn new_scan(&mut self) {
        self.target_pressed = false;
        self.invalid = false;
    }

    pub fn changed(&mut self, key: &KeyChange) {
        if key.x == self.target.0 && key.y == self.target.1 {
            if key.changed && key.state {
                if matches!(key.reg_as, Some(LayoutKey::None)) {
                    self.target_pressed = true;
                } else {
                    self.invalid = true;
                }
            }
        } else if let Some(layout_key) = key.reg_as {
            if key.state && key.changed && !layout_key.is_fn() {
                self.invalid = true;
            }
        }
    }

    pub fn apply(&mut self,
        bt_keyboard: &mut BleMouseKeyboard,
        display: &Display
    ) -> anyhow::Result<()> {
        if self.invalid {
            if self.press_count > 0 {
                display.clear()?;
            }

            self.press_count = 0;
            return Ok(());
        }

        if self.target_pressed {
            self.press_count += 1;
            info!("Unpair press count: {}", self.press_count);

            if self.press_count >= REQUIRED_PRESSES {
                self.press_count = 0;
                if bt_keyboard.connected() {
                    info!("Unpairing...");
                    display.show_text("Unpaired!")?;
                    bt_keyboard.unpair();
                } else {
                    info!("Not connected, advertising is active");
                    display.show_text("Advertising")?;
                }
            } else {
                let left = REQUIRED_PRESSES - self.press_count;
                self.buf.clear();
                let _ = write!(self.buf, "Unpair: {}", left);
                display.show_text(&self.buf)?;
            }
        }

        Ok(())
    }

    pub fn reset(&mut self) {
        self.press_count = 0;
    }
}
