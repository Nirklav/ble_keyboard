use std::rc::Rc;

use esp_idf_svc::hal::{delay::Delay, spi::{SpiDeviceDriver, SpiDriver}};
use log::info;
use mcp23s17::{InterruptMode, Mcp23s17};

use crate::layout::{Layout, LayoutKey, Layouts};

pub struct KeyMatrix<const W: usize, const H: usize> {
    ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
    delay: Delay,

    column_pins: [u8; W],
    row_pins: [u8; H],
    keys: [[Key; H]; W],
    current_layout: usize,
}

impl<const W: usize, const H: usize> KeyMatrix<W, H> {
    pub fn new(mut ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>, column_pins: [u8; W], row_pins: [u8; H]) -> anyhow::Result<Self> {

        for column in column_pins {
            map_err!(ext.pin_mode(column, mcp23s17::PinMode::Output));
        }

        for row in row_pins {
            map_err!(ext.pin_mode(row, mcp23s17::PinMode::InputPullup));
            map_err!(ext.set_interrupt_mode(row, InterruptMode::ActiveLow));
        }

        Ok(Self {
            ext,
            delay: Delay::new(5),
            column_pins,
            row_pins,
            keys: [[Key::new(); H]; W],
            current_layout: 0
        })
    }

    pub fn switch_layout<const C: usize>(&mut self, layouts: &'static Layouts<W, H, C>) -> &'static Layout<W, H> {
        let layout = layouts.get(self.current_layout);
        let layout_key = layout.fn_layout_key();
        if let Some(LayoutKey::Fn { permanent_switch, .. }) = layout_key {
            self.current_layout = *permanent_switch;
            info!("Layout switched to: {}", self.current_layout);
        }

        layouts.get(self.current_layout)
    }

    pub fn register<const C: usize, F>(&mut self, last_delay_ms: u32, layouts: &'static Layouts<W, H, C>, mut key_process: F) -> anyhow::Result<()> 
        where F: FnMut(KeyChange) {
        
        self.scan(last_delay_ms)?;

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

        for x in 0..W {
            for y in 0..H {
                let key = &mut self.keys[x][y];
                if key.state && key.changed {
                    if original_fn_coords == Some((x, y)) {
                        key.reg_as = Some(original_layout.get(x, y));
                    } else {
                        key.reg_as = Some(current_layout.get(x, y));
                    }
                }

                key_process(key.take_change());

                if !key.state {
                    key.reg_as = None;
                }
            }
        }

        Ok(())
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

    pub fn take_change(&mut self) -> KeyChange {
        let change = KeyChange { 
            state: self.state,
            changed: self.changed,
            reg_as: self.reg_as
        };
        self.changed = false;
        change
    }
}

#[derive(Clone, Copy)]
pub struct KeyChange {
    pub state: bool,
    pub changed: bool,
    pub reg_as: Option<&'static LayoutKey>
}