use std::rc::Rc;
use std::time::Instant;

use esp_idf_svc::hal::{delay::Delay, gpio::{Input, PinDriver}, spi::{SpiDeviceDriver, SpiDriver}};
use log::info;
use mcp23s17::{InterruptMode, Mcp23s17};

use protocol::config::KeyAction;

use crate::layout::{Keymap, Layout};

/// Keys behind the MCP23S17 extension, it pulls `irq` low when a row goes low.
pub struct KeyMatrix<const W: usize, const H: usize> {
    ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
    irq: PinDriver<'static, Input>,
    delay: Delay,

    column_pins: [u8; W],
    row_pins: [u8; H],
    keys: [[Key; H]; W],
    keymap: Keymap,
    current_layout: usize,
    last_scan: Instant,
}

impl<const W: usize, const H: usize> KeyMatrix<W, H> {
    pub fn new(
        mut ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
        irq: PinDriver<'static, Input>,
        column_pins: [u8; W],
        row_pins: [u8; H],
        keymap: Keymap,
    ) -> anyhow::Result<Self> {

        for column in column_pins {
            map_err!(ext.pin_mode(column, mcp23s17::PinMode::Output));
        }

        for row in row_pins {
            map_err!(ext.pin_mode(row, mcp23s17::PinMode::InputPullup));
            map_err!(ext.set_interrupt_mode(row, InterruptMode::ActiveLow));
        }

        Ok(Self {
            ext,
            irq,
            delay: Delay::new(5),
            column_pins,
            row_pins,
            keys: [[Key::new(); H]; W],
            keymap,
            current_layout: 0,
            last_scan: Instant::now(),
        })
    }

    pub fn irq(&self) -> &PinDriver<'static, Input> {
        &self.irq
    }

    pub fn irq_mut(&mut self) -> &mut PinDriver<'static, Input> {
        &mut self.irq
    }

    /// The first layout becomes current.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
        self.current_layout = 0;
        for key in self.keys.iter_mut().flatten() {
            key.reg_as = None;
        }
    }

    pub fn switch_layout(&mut self) -> &Layout {
        let layout = self.keymap.get(self.current_layout);
        if let Some(KeyAction::Fn { permanent, .. }) = layout.fn_action() {
            self.current_layout = permanent as usize;
            info!("Layout switched to: {}", self.current_layout);
        }

        self.keymap.get(self.current_layout)
    }

    /// `None` if there is no such layout.
    pub fn set_layout(&mut self, idx: usize) -> Option<&Layout> {
        if idx >= self.keymap.len() {
            return None;
        }

        self.current_layout = idx;
        info!("Layout set to: {}", self.current_layout);
        Some(self.keymap.get(idx))
    }

    /// Layout the keys are registered in now: the `Fn` temporary one while `Fn` is held.
    pub fn active_layout(&self) -> &Layout {
        active_layout(&self.keymap, self.current_layout, &self.keys)
    }

    /// Reads the key at `(x, y)` right now, without debouncing.
    pub fn is_pressed(&mut self, x: usize, y: usize) -> anyhow::Result<bool> {
        for column in self.column_pins {
            map_err!(self.ext.set_high(column));
        }

        map_err!(self.ext.set_low(self.column_pins[x]));
        self.delay.delay_us(5);
        let pressed = !map_err!(self.ext.read(self.row_pins[y]));

        for column in self.column_pins {
            map_err!(self.ext.set_low(column));
        }
        Ok(pressed)
    }

    /// Scans the matrix and passes every key to `key_process`.
    /// Returns `true` if any key is pressed.
    pub fn register<F>(&mut self, mut key_process: F) -> anyhow::Result<bool>
        where F: FnMut(KeyChange) {
        let elapsed_ms = self.last_scan.elapsed().as_millis() as u32;
        self.last_scan = Instant::now();
        self.scan(elapsed_ms)?;

        let original_layout = self.keymap.get(self.current_layout);
        let original_fn_coords = original_layout.fn_coord();
        let current_layout = active_layout(&self.keymap, self.current_layout, &self.keys);

        let mut any_pressed = false;
        for x in 0..W {
            for y in 0..H {
                let key = &mut self.keys[x][y];
                any_pressed |= key.state;
                if key.state && key.changed {
                    if original_fn_coords == Some((x, y)) {
                        key.reg_as = Some(original_layout.get(x, y));
                    } else {
                        key.reg_as = Some(current_layout.get(x, y));
                    }
                }

                key_process(key.take_change(x, y));

                if !key.state {
                    key.reg_as = None;
                }
            }
        }

        Ok(any_pressed)
    }

    fn scan(&mut self, last_delay_ms: u32) -> anyhow::Result<()> {
        for column in self.column_pins {
            map_err!(self.ext.set_high(column));
        }
        
        for (x, column) in self.column_pins.iter().enumerate() {
            map_err!(self.ext.set_low(*column));

            self.delay.delay_us(5);

            for (y, row) in self.row_pins.iter().enumerate() {
                let pressed = !map_err!(self.ext.read(*row));
                self.keys[x][y].set(pressed, last_delay_ms);
            }

            map_err!(self.ext.set_high(*column));
        }

        for column in self.column_pins {
            map_err!(self.ext.set_low(column));
        }
        Ok(())
    }
}

/// See [`KeyMatrix::active_layout`], borrows only the `keymap`, so the keys can be changed meanwhile.
fn active_layout<'a, const W: usize, const H: usize>(keymap: &'a Keymap, current: usize, keys: &[[Key; H]; W]) -> &'a Layout {
    let layout = keymap.get(current);
    if let Some((x, y)) = layout.fn_coord() {
        if keys[x][y].state {
            if let KeyAction::Fn { temporary, .. } = layout.get(x, y) {
                return keymap.get(temporary as usize);
            }
        }
    }
    layout
}

#[derive(Clone, Copy, Debug)]
struct Key {
    pub state: bool,
    reg_as: Option<KeyAction>,
    changed: bool,
    debounce_left_ms: u32
}

impl Key {
    pub fn new() -> Self {
        Key {
            state: false,
            reg_as: None,
            changed: false,
            debounce_left_ms: 0,
        }
    }

    pub fn set(&mut self, v: bool, ms: u32) {
        if self.debounce_left_ms > 0 {
            if self.debounce_left_ms < ms {
                self.debounce_left_ms = 0;
            } else {
                self.debounce_left_ms -= ms;
            }
        }

        if self.debounce_left_ms > 0 {
            return;
        }

        if self.state == v {
            return;
        }

        self.state = v;
        self.changed = true;
        self.debounce_left_ms = 5;
    }

    pub fn take_change(&mut self, x: usize, y: usize) -> KeyChange {
        let change = KeyChange {
            coord: Some((x as u8, y as u8)),
            state: self.state,
            reg_as: self.reg_as
        };
        self.changed = false;
        change
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeyChange {
    /// Matrix position `(x, y)`, `None` for a joystick button.
    pub coord: Option<(u8, u8)>,
    pub state: bool,
    pub reg_as: Option<KeyAction>
}