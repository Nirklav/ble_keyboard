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
const DEEP_SLEEP_DISCONNECTED_MS: u32 = 30_000;
const DEEP_SLEEP_INACTIVE_MS: u32 = 10 * 60 * 1000;

fn main() {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    
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

    prepare_deep_sleep(&ext_irq);

    let config = I2cConfig::new().baudrate(100.kHz().into());
    let i2c_driver = I2cDriver::new(i2c, sda, scl, &config)?;

    //let display = new_display(i2c_driver)?;

    let notification = Rc::new(Notification::new());
    setup_interrupt(&mut ext_irq, notification.clone())?;

    let mut ble_keyboard = BleMouseKeyboard::new()?;

    let spi_driver = Rc::new(SpiDriver::new(peripherals.spi2, sclk, mosi, Some(miso), &SpiDriverConfig::new())?);

    let ext = new_extension(spi_driver, ext_cs)?;
    let mut key_matrix = KeyMatrix::new(ext, [8, 9], [10, 11])?;
    
    let mut clear = true;
    let mut next_fn_switch = false;
    let layouts = &TEST;
 
    let delay = Delay::new_default();
    const DELAY_MS: u32 = 8;
    loop {
        delay.delay_ms(DELAY_MS);
        
        if clear {
            //map_err!(display.clear(Rgb565::BLACK));
        }

        let mut all_up = true;
        let mut only_fn_pressed = None;
        let mut send_key_report = false;
        let mut send_mouse_report = false;
        let mut send_media_key_report = false;

        key_matrix.register(DELAY_MS, layouts, |kc| {
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
                        _ => { }
                    }
                }
                
                if !kc.state && kc.changed && layout_key.is_fn() && only_fn_pressed.is_none() {
                    only_fn_pressed = Some(true);
                }
            }

            if kc.state {
                all_up = false;
            }
        })?;

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
            if next_fn_switch {
                key_matrix.switch_layout(layouts);
                next_fn_switch = false;
            } else {
                next_fn_switch = true;
            }
        }

        if all_up {
            ext_irq.enable_interrupt()?;

            if notification.wait(IDLE_DELAY_MS).is_none() {
                info!("idle");
                next_fn_switch = false;
                ble_keyboard.set_idle()?;

                let deep_sleep_timeout = if ble_keyboard.connected() {
                    DEEP_SLEEP_INACTIVE_MS
                } else {
                    DEEP_SLEEP_DISCONNECTED_MS
                };

                if notification.wait(deep_sleep_timeout).is_none() {
                    info!("entering deep sleep");
                    enter_deep_sleep(&ext_irq);
                }

                ble_keyboard.set_active()?;
                info!("active");
            }
        }
    }
}

fn prepare_deep_sleep(irq_pin: &PinDriver<'static, Input>) {
    unsafe { 
        esp_idf_sys::gpio_hold_dis(irq_pin.pin() as i32);
        esp_idf_sys::gpio_deep_sleep_hold_dis();
    }
}

fn enter_deep_sleep(irq_pin: &PinDriver<'static, Input>) {
    unsafe {
        // esp_idf_sys::gpio_sleep_set_pull_mode(
        //     gpio_num as i32,
        //     esp_idf_sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY,
        // );
        esp_idf_sys::gpio_hold_en(irq_pin.pin() as i32);
        esp_idf_sys::gpio_deep_sleep_hold_en();
        esp_idf_sys::esp_deep_sleep_enable_gpio_wakeup(
            1 << irq_pin.pin(),
            esp_idf_sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_LOW,
        );
        esp_idf_sys::esp_deep_sleep_start();
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