use std::rc::Rc;
use std::time::Instant;

use esp_idf_svc::hal::{delay::Ets, gpio::{Input, PinDriver}, spi::{SpiDeviceDriver, SpiDriver}};
use log::info;

/// The MCP23S17 extension the keys are behind, only what the scan needs: register pairs of both ports,
/// one SPI frame per access. Pin `n` is bit `n`: GPA0..7 are 0..7, GPB0..7 are 8..15.
pub struct Mcp23s17 {
    spi: SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>,
}

/// Register pairs with IOCON.BANK = 0: port A at the address, port B at the next one. A 2-byte access
/// goes A then B, IOCON.SEQOP = 0 increments the address.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Reg {
    IoDir = 0x00,
    GpIntEn = 0x04,
    DefVal = 0x06,
    IntCon = 0x08,
    GpPu = 0x0C,
    Gpio = 0x12,
    OLat = 0x14,
}

impl Mcp23s17 {
    /// Opcode `0100 A2 A1 A0 R/W`, A2..A0 are tied to GND.
    const WRITE: u8 = 0x40;
    const READ: u8 = 0x41;
    const IOCON: u8 = 0x0A;
    /// INTA and INTB are tied together on the board, so they must be open-drain (ODR, IO1 is pulled up),
    /// otherwise idle INTA drives the line high against active INTB. HAEN, BANK = 0, SEQOP = 0.
    const IOCON_VALUE: u8 = 0b0000_1100;

    pub fn new(spi: SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>) -> anyhow::Result<Self> {
        let mut ext = Self { spi };
        // HAEN is 0 after power on and 1 after a warm reboot: with A2..A0 at GND the address matches either way.
        ext.spi.write(&[Self::WRITE, Self::IOCON, Self::IOCON_VALUE])?;
        Ok(ext)
    }

    pub fn write(&mut self, reg: Reg, value: u16) -> anyhow::Result<()> {
        let [a, b] = value.to_le_bytes();
        self.spi.write(&[Self::WRITE, reg as u8, a, b])?;
        Ok(())
    }

    /// Reading [`Reg::Gpio`] clears a latched interrupt once its condition is gone.
    pub fn read(&mut self, reg: Reg) -> anyhow::Result<u16> {
        let mut bytes = [Self::READ, reg as u8, 0, 0];
        self.spi.transfer_in_place(&mut bytes)?;
        Ok(u16::from_le_bytes([bytes[2], bytes[3]]))
    }
}

/// From driving a column to reading the rows. A row released on the previous column rises through the
/// ~100 kΩ pull-up only (the diodes block the columns driven high): ~8..13 µs to a high level.
const SETTLE_US: u32 = 10;

/// Keys behind the MCP23S17 extension, it pulls `irq` low when a row goes low.
/// Only scans and debounces, what the keys do is up to [`protocol::keymap::Keymap`].
pub struct KeyMatrix<const W: usize, const H: usize> {
    ext: Mcp23s17,
    irq: PinDriver<'static, Input>,

    /// Bit of each column pin.
    column_masks: [u16; W],
    /// Bit of each row pin.
    row_masks: [u16; H],
    /// All the column bits.
    columns: u16,
    keys: [[Key; H]; W],
    last_scan: Instant,
    /// [`protocol::config::Settings::debounce_ms`].
    debounce_ms: u32,
    /// The slowest scan so far, logged when it grows.
    slowest_scan_us: u32,
}

impl<const W: usize, const H: usize> KeyMatrix<W, H> {
    pub fn new(
        mut ext: Mcp23s17,
        irq: PinDriver<'static, Input>,
        column_pins: [u8; W],
        row_pins: [u8; H],
    ) -> anyhow::Result<Self> {
        let column_masks = column_pins.map(|pin| 1u16 << pin);
        let row_masks = row_pins.map(|pin| 1u16 << pin);
        let columns = column_masks.iter().fold(0, |all, mask| all | mask);
        let rows = row_masks.iter().fold(0, |all, mask| all | mask);

        // The latches before the direction: the columns (and the unused pins) come up driving low, the idle state.
        ext.write(Reg::OLat, 0)?;
        ext.write(Reg::IoDir, rows)?;
        ext.write(Reg::GpPu, rows)?;
        // An interrupt while a row differs from DEFVAL (INTCON): a key held in idle keeps the line low.
        // GPINTEN last, so a half-set compare raises nothing.
        ext.write(Reg::DefVal, rows)?;
        ext.write(Reg::IntCon, rows)?;
        ext.write(Reg::GpIntEn, rows)?;
        // Clears an interrupt latched before a warm reboot, the MCP keeps its registers.
        ext.read(Reg::Gpio)?;

        Ok(Self {
            ext,
            irq,
            column_masks,
            row_masks,
            columns,
            keys: [[Key::default(); H]; W],
            last_scan: Instant::now(),
            debounce_ms: protocol::config::Settings::default().debounce_ms as u32,
            slowest_scan_us: 0,
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
        let port = self.read_column(x)?;
        self.ext.write(Reg::OLat, 0)?;
        Ok(port & self.row_masks[y] == 0)
    }

    /// Scans the matrix and passes every key that went down or up to `key_change` as `(x, y, pressed)`.
    /// Returns `true` if any key is pressed.
    pub fn register<F>(&mut self, mut key_change: F) -> anyhow::Result<bool>
        where F: FnMut(u8, u8, bool) {
        let elapsed_ms = self.last_scan.elapsed().as_millis() as u32;
        self.last_scan = Instant::now();
        self.scan(elapsed_ms)?;
        let scan_us = self.last_scan.elapsed().as_micros() as u32;
        if scan_us > self.slowest_scan_us {
            self.slowest_scan_us = scan_us;
            info!("Slowest key scan: {} us", scan_us);
        }

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
        for x in 0..W {
            let port = self.read_column(x)?;
            for (y, row) in self.row_masks.iter().enumerate() {
                self.keys[x][y].set(port & row == 0, last_delay_ms, self.debounce_ms);
            }
        }

        // Idle: all the columns low, a pressed key pulls its row and the interrupt line low.
        self.ext.write(Reg::OLat, 0)
    }

    /// Drives column `x` low and the others high, returns both ports. A high column can't pull a row
    /// (its diodes block), so only the keys of column `x` read low.
    fn read_column(&mut self, x: usize) -> anyhow::Result<u16> {
        // The row bits stay 0 in the latch, they're inputs: OLAT doesn't drive them.
        self.ext.write(Reg::OLat, self.columns & !self.column_masks[x])?;
        // Not `Delay`: from its threshold up it sleeps a whole FreeRTOS tick.
        Ets::delay_us(SETTLE_US);
        self.ext.read(Reg::Gpio)
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
