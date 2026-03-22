#[macro_use]
mod map_no_std_error;
mod storage;
mod bt;
mod key_matrix;

use std::rc::Rc;

use embedded_hal::spi::MODE_0;
use esp_idf_svc::hal::delay::Delay;
use esp_idf_svc::hal::gpio::*;
//use esp_idf_svc::hal::i2c::I2cConfig;
//use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::config::Config;
use esp_idf_svc::hal::spi::SpiDeviceDriver;
use esp_idf_svc::hal::spi::SpiDriver;
use esp_idf_svc::hal::spi::SpiDriverConfig;
use esp_idf_svc::hal::units::*;
use esp_idf_svc::log::EspLogger;
use log::info;
use mcp23s17::Mcp23s17;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
//use ssd1306::I2CDisplayInterface;
//use ssd1306::Ssd1306;
//use ssd1306::mode::DisplayConfig;
//use ssd1306::prelude::DisplayRotation;
//use ssd1306::size::DisplaySize128x32;
use crate::key_matrix::KeyMatrix;

fn main() {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    
    unsafe {
        esp_idf_sys::esp_wifi_stop();
        esp_idf_sys::esp_wifi_deinit();
    }

    main_body()
        .unwrap()
}

fn main_body() -> anyhow::Result<()> {
    storage::mount_vfs("/spiflash", "storage")?;

    let peripherals = Peripherals::take()?;

    let sclk = peripherals.pins.gpio6; // gpio6
    let miso = peripherals.pins.gpio4; // gpio2
    let mosi = peripherals.pins.gpio7; // gpio7
    let _ext_irq = PinDriver::input(peripherals.pins.gpio3, Pull::Down)?;
    let ext_cs = peripherals.pins.gpio9; // gpio8

    //let i2c = peripherals.i2c0;
    //let sda = peripherals.pins.gpio9;
    //let scl = peripherals.pins.gpio10;

    //let config = I2cConfig::new().baudrate(100.kHz().into());
    //let i2c = I2cDriver::new(i2c, sda, scl, &config)?;
    //let i2c_display = I2CDisplayInterface::new(i2c);

    //let mut display_delay = Ets;
    //let mut display = Ssd1306::new(i2c_display, DisplaySize128x32, DisplayRotation::Rotate0)
    //    .into_buffered_graphics_mode();
    //map_err!(display.init());

    let mut ble_keyboard = BleMouseKeyboard::new()?;

    let spi_driver = Rc::new(SpiDriver::new(peripherals.spi2, sclk, mosi, Some(miso), &SpiDriverConfig::new())?);
    let spi_ext_config = Config::new()
        .baudrate(10.MHz().into())
        .data_mode(MODE_0);
    let spi_ext_driver = SpiDeviceDriver::new(spi_driver, Some(ext_cs), &spi_ext_config)?;
    let mut ext = map_err!(Mcp23s17::new(spi_ext_driver, 0b000));
    map_err!(ext.pin_mode_all(mcp23s17::PinMode::Output));

    let mut key_matrix = KeyMatrix::new(ext, [8, 9], [10, 11])?;
    
    let mut clear = true;
 
    let delay = Delay::new(5);
    loop {
        delay.delay_ms(1);
        
        if clear {
            //map_err!(display.clear(Rgb565::BLACK));
        }

        key_matrix.scan(1)?;

        for x in 0..key_matrix.width() {
            for y in 0..key_matrix.height() {
                match key_matrix.take_change(x, y) {
                    Some(true) => {
                        info!("({}, {}) pressed", x, y);
                        
                    },
                    Some(false) => info!("({}, {}) released", x, y),
                    None => { }
                }
            }
        }
    }
}