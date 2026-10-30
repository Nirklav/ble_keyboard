//! Pins and drivers of the board: the key matrix behind an MCP23S17 on SPI, the display and the joystick on
//! the shared I2C bus, their interrupt pins.

use std::rc::Rc;

use embedded_hal::spi::MODE_0;
use esp_idf_svc::hal::gpio::{Gpio10, Gpio2, Gpio6, Gpio7, Gpio9, OutputPin, PinDriver, Pull};
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver, I2C0};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::config::Config;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriver, SpiDriverConfig, SPI2};
use esp_idf_svc::hal::task::notification::Notification;
use esp_idf_svc::hal::units::*;
use protocol::board;

use crate::display::Display;
use crate::i2c_bus::SharedI2c;
use crate::joystick::Joystick;
use crate::key_matrix::{KeyMatrix, Mcp23s17};
use crate::power;

pub const WIDTH: usize = board::WIDTH as usize;
pub const HEIGHT: usize = board::HEIGHT as usize;

/// The key matrix of the board.
pub type Matrix = KeyMatrix<WIDTH, HEIGHT>;

/// What [`init`] made of the peripherals.
pub struct Hardware {
    pub display: Display,
    pub key_matrix: Matrix,
    /// Notified by the interrupt pins, see [`power::setup_interrupt`].
    pub notification: Rc<Notification>,
    /// `None` if the joystick isn't connected.
    pub joystick: Option<Joystick>,
}

/// Sets the peripherals up, the interrupt pins wake the loop up and the chip from sleep.
pub fn init() -> anyhow::Result<Hardware> {
    let peripherals = Peripherals::take()?;

    let spi = peripherals.spi2;
    let sclk = peripherals.pins.gpio6;
    let miso = peripherals.pins.gpio2;
    let mosi = peripherals.pins.gpio7;
 
    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio9;
    let scl = peripherals.pins.gpio10;

    let ext_irq = PinDriver::input(peripherals.pins.gpio1, Pull::Up)?;
    let ext_cs = peripherals.pins.gpio8;
    // Pulled up, so a missing joystick reads as idle.
    let joystick_irq = PinDriver::input(peripherals.pins.gpio3, Pull::Up)?;

    let i2c = SharedI2c::new(new_i2c(i2c, sda, scl)?);
    let spi_driver = new_spi(spi, sclk, miso, mosi)?;

    let display = Display::new(i2c.clone())?;
    let ext = new_extension(spi_driver, ext_cs)?;
    let mut joystick = Joystick::new(i2c, joystick_irq);
    let mut key_matrix = Matrix::new(ext, ext_irq, [0, 1, 2, 3, 4, 5, 6, 7, 14, 15], [8, 9, 10, 11, 12])?;

    let pins = power::wakeup_pins(&key_matrix, &joystick);
    power::prepare_deep_sleep(&pins);
    power::prepare_light_sleep(&pins);
    
    let notification = Rc::new(Notification::new());
    power::setup_interrupt(key_matrix.irq_mut(), notification.clone())?;
    if let Some(joystick) = &mut joystick {
        power::setup_interrupt(joystick.irq_mut(), notification.clone())?;
    }

    Ok(Hardware { display, key_matrix, notification, joystick })
}

fn new_i2c(i2c: I2C0<'static>, sda: Gpio9<'static>, scl: Gpio10<'static>) -> anyhow::Result<I2cDriver<'static>> {
    // A display frame takes ~13 ms, at 100 kHz pop-up slides would stutter; the joystick suits both.
    let config = I2cConfig::new().baudrate(400.kHz().into());
    let driver = I2cDriver::new(i2c, sda, scl, &config)?;
    Ok(driver)
}

fn new_spi(spi: SPI2<'static>, sclk: Gpio6<'static>, miso: Gpio2<'static>, mosi: Gpio7<'static>) -> anyhow::Result<Rc<SpiDriver<'static>>> {
    let config = SpiDriverConfig::new();
    let spi_driver = Rc::new(SpiDriver::new(spi, sclk, mosi, Some(miso), &config)?);
    Ok(spi_driver)
}

fn new_extension(spi_driver: Rc<SpiDriver<'static>>, cs: impl OutputPin + 'static) -> anyhow::Result<Mcp23s17> {
    let spi_ext_config = Config::new()
        .baudrate(10.MHz().into())
        .data_mode(MODE_0);
    let mut spi_ext_driver = SpiDeviceDriver::new(spi_driver, Some(cs), &spi_ext_config)?;

    // INTA and INTB are tied together on the board, so they must be open-drain (IO1 is pulled up),
    // otherwise idle INTA drives the line high against active INTB. The crate has no API for IOCON,
    // so it's written directly; `Mcp23s17::new` keeps the bit when it sets HAEN.
    const MCP_WRITE_OPCODE: u8 = 0x40;
    spi_ext_driver.write(&[
        MCP_WRITE_OPCODE,
        mcp23s17::RegisterAddress::IOCON as u8,
        mcp23s17::IOCON::ODR_ON.bits(),
    ])?;

    let mut ext = map_err!(Mcp23s17::new(spi_ext_driver, 0b000));
    map_err!(ext.pin_mode_all(mcp23s17::PinMode::Output));
    Ok(ext)
}
