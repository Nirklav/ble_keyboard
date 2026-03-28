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
use crate::layout::LayoutKey;
use crate::layout::{TEST, DEFAULT};
use ssd1306::I2CDisplayInterface;
use ssd1306::Ssd1306;
use ssd1306::mode::DisplayConfig;
use ssd1306::prelude::DisplayRotation;
use ssd1306::size::DisplaySize128x32;
use crate::key_matrix::KeyMatrix;

const IDLE_DELAY_MS: u32 = 500;

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
    let mut next_fn_swith = false;
    let mut idle_timer: u32 = 0;
    let layouts = &TEST;
 
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
            ext_irq.enable_interrupt()?;
            notification.wait(esp_idf_svc::hal::delay::BLOCK);
            active = true;
            ble_keyboard.set_active()?;
            info!("active");
        }

        key_matrix.scan(DELAY_MS)?;
        key_matrix.register_changed(layouts);

        let mut all_up = true;
        let mut only_fn_pressed = None;
        let mut send_key_report = false;
        let mut send_mouse_report = false;
        let mut send_media_key_report = false;

        for x in 0..key_matrix.width() {
            for y in 0..key_matrix.height() {

                let kc = key_matrix.take_change(x, y);                
                if let Some(layout_key) = kc.reg_as {
                    if kc.changed {
                        match layout_key {
                            LayoutKey::Key(_) | LayoutKey::KeyWithMod(_, _) | LayoutKey::Mod(_) => send_key_report = true,
                            LayoutKey::Media(_) => send_media_key_report = true,
                            LayoutKey::MouseKey(_) => send_mouse_report = true,
                            _ => { }
                        }
                    }

                    if kc.state {
                        if !layout_key.is_fn() {
                            only_fn_pressed = Some(false);
                        }   

                        match layout_key {
                            LayoutKey::Key(key) => ble_keyboard.press_key(*key),
                            LayoutKey::KeyWithMod(key, key_mod) => {
                                ble_keyboard.press_key_mod(*key_mod);
                                ble_keyboard.press_key(*key);
                            },
                            LayoutKey::Mod(key_mod) => ble_keyboard.press_key_mod(*key_mod),
                            LayoutKey::Media(media_key) => ble_keyboard.press_media_key(*media_key),
                            LayoutKey::MouseKey(mouse_key) => ble_keyboard.press_mouse(*mouse_key),
                            LayoutKey::MouseTravel { dx, dy } => {
                                send_mouse_report = true;
                                ble_keyboard.travel_mouse_cursor(*dx, *dy)
                            },
                            LayoutKey::MouseScroll { dy } => {
                                send_mouse_report = true;
                                ble_keyboard.travel_mouse_wheel(*dy)
                            },
                            LayoutKey::Fn { .. } => { },
                            LayoutKey::None => { }
                        }
                    }
                    
                    if !kc.state && kc.changed && layout_key.is_fn() && only_fn_pressed.is_none() {
                        only_fn_pressed = Some(true);
                    }
                }

                if kc.state {
                    all_up = false;
                }
            }
        }

        key_matrix.unregister_changed();

        if ble_keyboard.connected() {
            if send_key_report {
                ble_keyboard.send_key_report();
            }

            if send_mouse_report {
                ble_keyboard.send_mouse_report();
            }

            if send_media_key_report {
                ble_keyboard.send_media_key_report();
            }
        }

        ble_keyboard.clear_keys();
        ble_keyboard.clear_mouse();
        ble_keyboard.clear_media_key();

        if let Some(true) = only_fn_pressed {
            if next_fn_swith {
                key_matrix.switch_layout(layouts);
                next_fn_swith = false;
            } else {
                next_fn_swith = true;
            }
        }

        if all_up {
            idle_timer += DELAY_MS;
            if idle_timer >= IDLE_DELAY_MS {
                active = false;
                next_fn_swith = false;
            }
        } else {
            idle_timer = 0;
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