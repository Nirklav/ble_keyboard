use std::rc::Rc;
use std::time::Instant;

use esp_idf_svc::hal::{delay::Delay, gpio::{Input, PinDriver}, spi::{SpiDeviceDriver, SpiDriver}};
use mcp23s17::{InterruptMode, Mcp23s17};

/// Keys behind the MCP23S17 extension, it pulls `irq` low when a row goes low.
/// Only scans and debounces, what the keys do is up to [`protocol::keymap::Keymap`].
pub struct KeyMatrix<const W: usize, const H: usize> {
    ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
    irq: PinDriver<'static, Input>,
    delay: Delay,

    column_pins: [u8; W],
    row_pins: [u8; H],
    keys: [[Key; H]; W],
    last_scan: Instant,
    /// [`protocol::config::Settings::debounce_ms`].
    debounce_ms: u32,
}

impl<const W: usize, const H: usize> KeyMatrix<W, H> {
    pub fn new(
        mut ext: Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>,
        irq: PinDriver<'static, Input>,
        column_pins: [u8; W],
        row_pins: [u8; H],
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
            keys: [[Key::default(); H]; W],
            last_scan: Instant::now(),
            debounce_ms: protocol::config::Settings::default().debounce_ms as u32,
        })
    }

    /// A key change is ignored for `ms` after the previous one.
    pub fn set_debounce_ms(&mut self, ms: u16) {
        self.debounce_ms = ms as u32;
    }

    pub fn irq(&self) -> &PinDriver<'static, Input> {
        &self.irq
    }

    pub fn irq_mut(&mut self) -> &mut PinDriver<'static, Input> {
        &mut self.irq
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

    /// Scans the matrix and passes every key that went down or up to `key_change` as `(x, y, pressed)`.
    /// Returns `true` if any key is pressed.
    pub fn register<F>(&mut self, mut key_change: F) -> anyhow::Result<bool>
        where F: FnMut(u8, u8, bool) {
        let elapsed_ms = self.last_scan.elapsed().as_millis() as u32;
        self.last_scan = Instant::now();
        self.scan(elapsed_ms)?;

        let mut any_pressed = false;
        for x in 0..W {
            for y in 0..H {
                let key = &mut self.keys[x][y];
                any_pressed |= key.state;
                if key.changed {
                    key.changed = false;
                    key_change(x as u8, y as u8, key.state);
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
                self.keys[x][y].set(pressed, last_delay_ms, self.debounce_ms);
            }

            map_err!(self.ext.set_high(*column));
        }

        for column in self.column_pins {
            map_err!(self.ext.set_low(column));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Key {
    state: bool,
    changed: bool,
    debounce_left_ms: u32
}

impl Key {
    fn set(&mut self, v: bool, ms: u32, debounce_ms: u32) {
        self.debounce_left_ms = self.debounce_left_ms.saturating_sub(ms);
        if self.debounce_left_ms > 0 || self.state == v {
            return;
        }

        self.state = v;
        self.changed = true;
        self.debounce_left_ms = debounce_ms;
    }
}
