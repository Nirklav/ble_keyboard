 #[macro_use]
mod map_no_std_error;
mod storage;
mod bt;
mod key_matrix;
mod layout;
mod display;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use embedded_hal::spi::MODE_0;
use esp_idf_svc::hal::delay::Delay;
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::*;
use esp_idf_svc::hal::i2c::I2C0;
use esp_idf_svc::hal::i2c::I2cConfig;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::SPI2;
use esp_idf_svc::hal::spi::config::Config;
use esp_idf_svc::hal::spi::SpiDeviceDriver;
use esp_idf_svc::hal::spi::SpiDriver;
use esp_idf_svc::hal::spi::SpiDriverConfig;
use esp_idf_svc::hal::task::notification::Notification;
use esp_idf_svc::hal::units::*;
use esp_idf_svc::log::EspLogger;
use log::info;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::display::Display;
use crate::layout::LayoutKey;
use crate::layout::DEFAULT;
use crate::key_matrix::KeyMatrix;

type Mcp23s17 = mcp23s17::Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>;

const IDLE_DELAY_TICKS: TickType = TickType::new_millis(1000);
const DEEP_SLEEP_DISCONNECTED_TICKS: TickType = TickType::new_millis(30_000);
const DEEP_SLEEP_INACTIVE_TICKS: TickType = TickType::new_millis(5 * 60 * 1000);
const SCREEN_TEXT_TIMEOUT: Duration = Duration::from_secs(5);

fn main() {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    
    main_body()
        .unwrap()
}

fn main_body() -> anyhow::Result<()> {
    storage::mount_vfs("/spiflash", "storage")?;

    let (display, mut key_matrix, notification, mut ext_irq) = peripherals()?;
    
    let mut ble_keyboard = BleMouseKeyboard::new()?;
    let delay = Delay::new_default();

    let mut since_sleep = Instant::now();
    let layouts = &DEFAULT;
    let mut next_fn_switch = false;
    
    display.show_text_for("Welcome!", SCREEN_TEXT_TIMEOUT)?;
    const DELAY_MS: u32 = 8;
    loop {
        let mut all_up = true;
        let mut only_fn_pressed = None;
        let mut send_key_report = false;
        let mut send_mouse_report = false;
        let mut send_media_key_report = false;

        let elaped_ms = since_sleep.elapsed().as_millis() as u32;
        key_matrix.register(elaped_ms, layouts, |kc| {
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
                let new_layout = key_matrix.switch_layout(layouts);
                display.show_text_for(new_layout.name(), SCREEN_TEXT_TIMEOUT)?;
                next_fn_switch = false;
            } else {
                next_fn_switch = true;
            }
        }

        since_sleep = Instant::now();
        if all_up {
            ext_irq.enable_interrupt()?;
            if notification.wait(IDLE_DELAY_TICKS.into()).is_none() {
                info!("idle");
                next_fn_switch = false;
                ble_keyboard.set_idle()?;

                let deep_sleep_timeout = if ble_keyboard.connected() {
                    DEEP_SLEEP_INACTIVE_TICKS
                } else {
                    DEEP_SLEEP_DISCONNECTED_TICKS
                };

                if notification.wait(deep_sleep_timeout.into()).is_none() {
                    info!("entering deep sleep");
                    enter_deep_sleep(&ext_irq);
                }

                ble_keyboard.set_active()?;
                info!("active");
            }
        } else {
            delay.delay_ms(DELAY_MS);
        }
    }
}

fn peripherals() -> anyhow::Result<(Display, KeyMatrix<10, 5>, Rc<Notification>, PinDriver<'static, Input>)> {
    let peripherals = Peripherals::take()?;

    let spi = peripherals.spi2;
    let sclk = peripherals.pins.gpio6;
    let miso = peripherals.pins.gpio2;
    let mosi = peripherals.pins.gpio7;
 
    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio9;
    let scl = peripherals.pins.gpio10;

    let mut ext_irq = PinDriver::input(peripherals.pins.gpio1, Pull::Up)?;
    let ext_cs = peripherals.pins.gpio8;

    let i2c_driver = new_i2c(i2c, sda, scl)?;
    let spi_driver = new_spi(spi, sclk, miso, mosi)?;

    let display = Display::new(i2c_driver)?;
    let ext = new_extension(spi_driver, ext_cs)?;

    prepare_deep_sleep(&ext_irq);
    prepare_light_sleep(&ext_irq);
    
    let notification = Rc::new(Notification::new());
    setup_interrupt(&mut ext_irq, notification.clone())?;

    let key_matrix = KeyMatrix::new(ext, [0, 1, 2, 3, 4, 5, 6, 7, 14, 15], [8, 9, 10, 11, 12])?;

    Ok((display, key_matrix, notification, ext_irq))
}

fn new_i2c(i2c: I2C0<'static>, sda: Gpio9<'static>, scl: Gpio10<'static>) -> anyhow::Result<I2cDriver<'static>> {
    let config = I2cConfig::new().baudrate(100.kHz().into());
    let driver = I2cDriver::new(i2c, sda, scl, &config)?;
    Ok(driver)
}

fn new_spi(spi: SPI2<'static>, sclk: Gpio6<'static>, miso: Gpio2<'static>, mosi: Gpio7<'static>) -> anyhow::Result<Rc<SpiDriver<'static>>> {
    let config = SpiDriverConfig::new();
    let spi_driver = Rc::new(SpiDriver::new(spi, sclk, mosi, Some(miso), &config)?);
    Ok(spi_driver)
}

fn prepare_light_sleep(irq_pin: &PinDriver<'static, Input>) {
    unsafe {
        esp_idf_sys::gpio_wakeup_enable(
            irq_pin.pin() as i32,
            esp_idf_sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL,
        );
        esp_idf_sys::esp_sleep_enable_gpio_wakeup();
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
    ext_irq.set_interrupt_type(InterruptType::LowLevel)?;

    let pin = ext_irq.pin();
    let notifier = notification.notifier();
    unsafe {
        ext_irq.subscribe(move || {
            notifier.notify_and_yield(NonZeroU32::new(1).unwrap());
            esp_idf_sys::gpio_intr_disable(pin as i32);
        })?;
    }
    Ok(())
}

fn new_extension(spi_driver: Rc<SpiDriver<'static>>, cs: impl OutputPin + 'static) -> anyhow::Result<Mcp23s17> {
    let spi_ext_config = Config::new()
        .baudrate(10.MHz().into())
        .data_mode(MODE_0);
    let spi_ext_driver = SpiDeviceDriver::new(spi_driver, Some(cs), &spi_ext_config)?;
    let mut ext = map_err!(Mcp23s17::new(spi_ext_driver, 0b000));
    map_err!(ext.pin_mode_all(mcp23s17::PinMode::Output));
    Ok(ext)
}