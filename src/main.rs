#[macro_use]
mod map_no_std_error;
mod storage;
mod bt;
mod key_matrix;
mod layout;

use std::num::NonZeroU32;
use std::rc::Rc;

use embedded_hal::spi::MODE_0;
use esp_idf_svc::hal::delay::Delay;
use esp_idf_svc::hal::gpio::*;
use esp_idf_svc::hal::i2c::I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::reset::ResetReason;
use esp_idf_svc::hal::spi::config::Config;
use esp_idf_svc::hal::spi::SpiDeviceDriver;
use esp_idf_svc::hal::spi::SpiDriver;
use esp_idf_svc::hal::spi::SpiDriverConfig;
use esp_idf_svc::hal::task::notification::Notification;
use esp_idf_svc::hal::units::*;
use esp_idf_svc::log::EspLogger;
use log::info;
use mcp23s17::Mcp23s17;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::I2CInterface;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::key_matrix::KeyChange;
use ssd1306::I2CDisplayInterface;
use ssd1306::Ssd1306;
use ssd1306::mode::DisplayConfig;
use ssd1306::prelude::DisplayRotation;
use ssd1306::size::DisplaySize128x32;
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
    let mut ext_irq = PinDriver::input(peripherals.pins.gpio8, Pull::Up)?; // gpio3
    let ext_cs = peripherals.pins.gpio9; // gpio8

    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio2; // gpio9
    let scl = peripherals.pins.gpio3; // gpio10

    let config = I2cConfig::new().baudrate(100.kHz().into());
    let i2c_driver = I2cDriver::new(i2c, sda, scl, &config)?;

    //let display = new_display(i2c_driver)?;

    let notification = Rc::new(Notification::new());
    setup_interrupt(&mut ext_irq, notification.clone())?;

    let mut ble_keyboard = BleMouseKeyboard::new()?;

    let reset_reason = ResetReason::get();

    let spi_driver = Rc::new(SpiDriver::new(peripherals.spi2, sclk, mosi, Some(miso), &SpiDriverConfig::new())?);

    let ext = new_extension(spi_driver, ext_cs)?;
    let mut key_matrix = KeyMatrix::new(ext, [8, 9], [10, 11])?;
    
    let mut clear = true;
    let mut active = true;
 
    let delay = Delay::new(5);
    const DELAY_MS: u32 = 8;
    loop {
        delay.delay_ms(DELAY_MS);
        
        if clear {
            //map_err!(display.clear(Rgb565::BLACK));
        }

        if !active {
            info!("idle");
            ble_keyboard.set_idle()?;
            notification.wait(esp_idf_svc::hal::delay::BLOCK);
            active = true;
            ble_keyboard.set_active()?;
            info!("active");
        }

        key_matrix.scan(DELAY_MS)?;

        let mut all_up = true;
        for x in 0..key_matrix.width() {
            for y in 0..key_matrix.height() {
                let kc = key_matrix.take_change(x, y);
                match &kc {
                    KeyChange { state: true, changed: true } => {
                       info!("({}, {}) pressed", x, y); 
                    },
                    KeyChange { state: false, changed: true } => {
                        info!("({}, {}) released", x, y);
                    }
                    _ => { }
                }

                if kc.state {
                    all_up = false;
                }
            }
        }

        if all_up {
            active = false;
        }
    }
}

fn setup_interrupt(ext_irq: &mut PinDriver<'static, Input>, notification: Rc<Notification>) -> anyhow::Result<()> {
    ext_irq.set_interrupt_type(InterruptType::NegEdge)?;
    
    let notifier = notification.notifier();
    unsafe {
        ext_irq.subscribe(move || {
            notifier.notify_and_yield(NonZeroU32::new(1).unwrap());
        })?;
    }
    ext_irq.enable_interrupt()?;
    Ok(())
}

fn new_display(i2c_driver: I2cDriver<'static>) -> anyhow::Result<Ssd1306<I2CInterface<I2cDriver<'static>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>> {
    let i2c_display = I2CDisplayInterface::new(i2c_driver);

    let mut display = Ssd1306::new(i2c_display, DisplaySize128x32, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    map_err!(display.init());

    Ok(display)
}

fn new_extension(spi_driver: Rc<SpiDriver<'static>>, cs: impl OutputPin + 'static) -> anyhow::Result<Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>> {
    let spi_ext_config = Config::new()
        .baudrate(10.MHz().into())
        .data_mode(MODE_0);
    let spi_ext_driver = SpiDeviceDriver::new(spi_driver, Some(cs), &spi_ext_config)?;
    let mut ext = map_err!(Mcp23s17::new(spi_ext_driver, 0b000));
    map_err!(ext.pin_mode_all(mcp23s17::PinMode::Output));
    Ok(ext)
}