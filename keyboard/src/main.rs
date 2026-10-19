 #[macro_use]
mod map_no_std_error;
mod config;
mod config_store;
mod storage;
mod bt;
mod key_matrix;
mod layout;
mod display;
mod i2c_bus;
mod joystick;
mod sc;
mod services;

use std::num::NonZeroU32;
use std::rc::Rc;

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
use crate::i2c_bus::SharedI2c;
use crate::joystick::Joystick;
use crate::layout::Keymap;
use crate::key_matrix::KeyMatrix;
use crate::sc::SpecialCombinations;
use crate::services::Services;

type Mcp23s17 = mcp23s17::Mcp23s17<SpiDeviceDriver<'static, Rc<SpiDriver<'static>>>>;

const WIDTH: usize = 10;
const HEIGHT: usize = 5;

/// Held at power-on, ignores the saved config: `Escape`.
const BOOT_KEY: (usize, usize) = (0, 0);

const IDLE_DELAY_TICKS: TickType = TickType::new_millis(1000);
const DEEP_SLEEP_DISCONNECTED_TICKS: TickType = TickType::new_millis(30_000);
const DEEP_SLEEP_INACTIVE_TICKS: TickType = TickType::new_millis(5 * 60 * 1000);

fn main() {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    
    main_body()
        .unwrap()
}

fn main_body() -> anyhow::Result<()> {
    storage::mount_vfs("/spiflash", "storage")?;

    let (display, mut key_matrix, notification, mut joystick) = peripherals(Keymap::from_config(&layout::default_config()))?;

    // Escape also wakes the keyboard up from deep sleep, that's not a request to bypass.
    let bypass = !woke_from_deep_sleep() && key_matrix.is_pressed(BOOT_KEY.0, BOOT_KEY.1)?;
    let (config, source) = config::load(WIDTH as u8, HEIGHT as u8, bypass);
    if let Err(e) = config.validate_for(WIDTH as u8, HEIGHT as u8) {
        panic!("config is invalid: {}", e);
    }
    key_matrix.set_keymap(Keymap::from_config(&config));
    
    let keyboard = BleMouseKeyboard::new(WIDTH as u8, HEIGHT as u8, config.to_bytes(), notification.notifier())?;
    let delay = Delay::new_default();

    let mut special_combinations = SpecialCombinations::from_config(&config.combos);
    let mut svc = Services::new(keyboard, key_matrix, display, config);

    svc.display.show_text(source.message().unwrap_or("Welcome!"))?;

    const DELAY_MS: u32 = 8;
    loop {
        let mut all_up = !svc.key_matrix.register(|kc| {
            if let (true, Some(action)) = (kc.state, kc.reg_as) {
                layout::press(&action, &mut svc.keyboard);
            }
            special_combinations.changed(&kc);
        })?;

        if let Some(joystick) = &mut joystick {
            all_up &= !joystick.poll(&mut svc.keyboard);
        }

        svc.keyboard.flush();
        special_combinations.try_apply(&mut svc)?;

        if all_up {
            config::apply_changes(&mut svc, &mut special_combinations)?;
            wait_for_activity(&notification, &mut svc, &mut joystick, &mut special_combinations)?;
        } else {
            delay.delay_ms(DELAY_MS);
        }
    }
}

fn wait_for_activity<const W: usize, const H: usize>(
    notification: &Notification,
    svc: &mut Services<W, H>,
    joystick: &mut Option<Joystick>,
    special_combinations: &mut SpecialCombinations,
) -> anyhow::Result<()> {
    svc.key_matrix.irq_mut().enable_interrupt()?;
    if let Some(joystick) = joystick {
        joystick.irq_mut().enable_interrupt()?;
    }

    // Config requests wake the loop up too, a submitted config is applied on return.
    if notification.wait(IDLE_DELAY_TICKS.into()).is_some() {
        return Ok(());
    }

    // Idle connection parameters (slave latency) would slow a config transfer down.
    while svc.keyboard.config_service().transfer_active() {
        if notification.wait(IDLE_DELAY_TICKS.into()).is_some() {
            return Ok(());
        }
    }

    info!("idle");
    special_combinations.reset();
    svc.keyboard.set_idle()?;

    let deep_sleep_timeout = if svc.keyboard.connected() {
        DEEP_SLEEP_INACTIVE_TICKS
    } else {
        DEEP_SLEEP_DISCONNECTED_TICKS
    }
    .into();

    // No deep sleep while the config app is connected; its pings don't wake the loop up,
    // so the keyboard stays idle and checks again after another timeout.
    while notification.wait(deep_sleep_timeout).is_none() {
        if !svc.keyboard.config_service().session_active() {
            info!("entering deep sleep");
            enter_deep_sleep(&wakeup_pins(&svc.key_matrix, joystick));
        }
        info!("deep sleep postponed, the config app is connected");
    }

    svc.keyboard.set_active()?;
    info!("active");
    Ok(())
}

fn woke_from_deep_sleep() -> bool {
    unsafe { esp_idf_sys::esp_reset_reason() == esp_idf_sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP }
}

fn peripherals(keymap: Keymap) -> anyhow::Result<(Display, KeyMatrix<WIDTH, HEIGHT>, Rc<Notification>, Option<Joystick>)> {
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
    let mut key_matrix = KeyMatrix::new(ext, ext_irq, [0, 1, 2, 3, 4, 5, 6, 7, 14, 15], [8, 9, 10, 11, 12], keymap)?;

    let pins = wakeup_pins(&key_matrix, &joystick);
    prepare_deep_sleep(&pins);
    prepare_light_sleep(&pins);
    
    let notification = Rc::new(Notification::new());
    setup_interrupt(key_matrix.irq_mut(), notification.clone())?;
    if let Some(joystick) = &mut joystick {
        setup_interrupt(joystick.irq_mut(), notification.clone())?;
    }

    Ok((display, key_matrix, notification, joystick))
}

/// Interrupt pins that wake the keyboard up, all active low.
fn wakeup_pins<'a, const W: usize, const H: usize>(
    key_matrix: &'a KeyMatrix<W, H>,
    joystick: &'a Option<Joystick>,
) -> Vec<&'a PinDriver<'static, Input>> {
    let mut pins = vec![key_matrix.irq()];
    if let Some(joystick) = joystick {
        pins.push(joystick.irq());
    }
    pins
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

fn prepare_light_sleep(irq_pins: &[&PinDriver<'static, Input>]) {
    unsafe {
        for irq_pin in irq_pins {
            esp_idf_sys::gpio_wakeup_enable(
                irq_pin.pin() as i32,
                esp_idf_sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL,
            );
        }
        esp_idf_sys::esp_sleep_enable_gpio_wakeup();
    }
}

fn prepare_deep_sleep(irq_pins: &[&PinDriver<'static, Input>]) {
    unsafe { 
        for irq_pin in irq_pins {
            esp_idf_sys::gpio_hold_dis(irq_pin.pin() as i32);
        }
        esp_idf_sys::gpio_deep_sleep_hold_dis();
    }
}

fn enter_deep_sleep(irq_pins: &[&PinDriver<'static, Input>]) {
    unsafe {
        let mut mask = 0;
        for irq_pin in irq_pins {
            esp_idf_sys::gpio_hold_en(irq_pin.pin() as i32);
            mask |= 1 << irq_pin.pin();
        }
        esp_idf_sys::gpio_deep_sleep_hold_en();
        esp_idf_sys::esp_deep_sleep_enable_gpio_wakeup(
            mask,
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