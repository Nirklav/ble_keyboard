use std::rc::Rc;
use std::time::Instant;

use esp_idf_svc::hal::{delay::Delay, gpio::{Input, PinDriver}, spi::{SpiDeviceDriver, SpiDriver}};
use log::info;
use mcp23s17::{InterruptMode, Mcp23s17};

use crate::layout::{Layout, LayoutKey, Layouts};

/// Keys behind the MCP23S17 extension, it pulls `irq` low when a row goes low.
pub struct KeyMatrix<const W: usize, const H: usize, const C: usize> {
    ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
    irq: PinDriver<'static, Input>,
    delay: Delay,

    column_pins: [u8; W],
    row_pins: [u8; H],
    keys: [[Key; H]; W],
    layouts: &'static Layouts<W, H, C>,
    current_layout: usize,
    last_scan: Instant,
}

impl<const W: usize, const H: usize, const C: usize> KeyMatrix<W, H, C> {
    pub fn new(
        mut ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
        irq: PinDriver<'static, Input>,
        column_pins: [u8; W],
        row_pins: [u8; H],
        layouts: &'static Layouts<W, H, C>,
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
            layouts,
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

    pub fn switch_layout(&mut self) -> &'static Layout<W, H> {
        let layouts = self.layouts;
        let layout = layouts.get(self.current_layout);
        let layout_key = layout.fn_layout_key();
        if let Some(LayoutKey::Fn { permanent_switch, .. }) = layout_key {
            self.current_layout = *permanent_switch;
            info!("Layout switched to: {}", self.current_layout);
        }

        layouts.get(self.current_layout)
    }

    /// Scans the matrix and passes every key to `key_process`.
    /// Returns `true` if any key is pressed.
    pub fn register<F>(&mut self, mut key_process: F) -> anyhow::Result<bool>
        where F: FnMut(KeyChange) {
        let layouts = self.layouts;

        let elapsed_ms = self.last_scan.elapsed().as_millis() as u32;
        self.last_scan = Instant::now();
        self.scan(elapsed_ms)?;

        let original_layout = layouts.get(self.current_layout);
        let mut current_layout = original_layout;

        let original_fn_coords = current_layout.fn_coords();
        if let Some((x, y)) = original_fn_coords {
            if self.keys[x][y].state {
                if let LayoutKey::Fn { temporary_switch, .. } = &current_layout.get(x, y) {
                    current_layout = layouts.get(*temporary_switch)
                }
            }
        }

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

#[derive(Clone, Copy, Debug)]
struct Key {
    pub state: bool,
    reg_as: Option<&'static LayoutKey>,
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
            x: x as u8,
            y: y as u8,
            state: self.state,
            reg_as: self.reg_as
        };
        self.changed = false;
        change
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeyChange {
    pub x: u8,
    pub y: u8,
    pub state: bool,
    pub reg_as: Option<&'static LayoutKey>
}