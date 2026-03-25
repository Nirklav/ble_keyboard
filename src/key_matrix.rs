use std::rc::Rc;

use esp_idf_svc::hal::{delay::Delay, spi::{SpiDeviceDriver, SpiDriver}};
use mcp23s17::Mcp23s17;

pub struct KeyMatrix<const W: usize, const H: usize> {
    ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
    delay: Delay,

    column_pins: [u8; W],
    row_pins: [u8; H],
    keys: [[Key; H]; W],
}

impl<const W: usize, const H: usize> KeyMatrix<W, H> {
    pub fn new(mut ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>, column_pins: [u8; W], row_pins: [u8; H]) -> anyhow::Result<Self> {

        for column in column_pins {
            map_err!(ext.pin_mode(column, mcp23s17::PinMode::Output));
        }

        for row in row_pins {
            map_err!(ext.pin_mode(row, mcp23s17::PinMode::InputPullup));
        }

        Ok(Self {
            ext,
            delay: Delay::new(5),
            column_pins,
            row_pins,
            keys: [[Key::new(); H]; W]
        })
    }

    pub fn width(&self) -> usize {
        W
    }

    pub fn height(&self) -> usize {
        H
    }

    pub fn take_change(&mut self, x: usize, y: usize) -> KeyChange {
        self.keys[x][y].take_change()
    }

    pub fn scan(&mut self, last_delay_ms: u32) -> anyhow::Result<()> {
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

#[derive(Clone, Copy)]
struct Key {
    pub state: bool,
    changed: bool,
    debounce_left_ms: u32
}

impl Key {
    pub fn new() -> Self {
        Key {
            state: false,
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
            changed: self.changed
        };
        self.changed = false;
        change
    }
}

#[derive(Clone, Copy)]
pub struct KeyChange {
    pub state: bool, 
    pub changed: bool 
}