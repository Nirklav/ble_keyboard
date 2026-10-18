//! Joystick extension: an analog stick with a press and two buttons, read by the
//! keyboard over I2C (see [`protocol::joystick`]).
//!
//! Pins (see `hardware/schematic.pdf`):
//! - PA0: stick potentiometers supply
//! - PA1, PA2: stick X, Y (ADC)
//! - PA3: stick press, PA4: left button, PA5: right button (to GND)
//! - PA6: `INT` to the keyboard
//! - PA9, PA10: I2C1 SCL, SDA (pull-ups are on the keyboard)
#![no_main]
#![no_std]

mod i2c_slave;
mod input;

use panic_halt as _;
use stm32f0xx_hal as hal;

use cortex_m::peripheral::Peripherals;
use cortex_m_rt::entry;
use embedded_hal::adc::{Channel, OneShot};

use crate::hal::adc::Adc;
use crate::hal::delay::Delay;
use crate::hal::watchdog::Watchdog;
use crate::hal::{pac, prelude::*};

use protocol::joystick::{State, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_STICK};

use crate::input::{Axis, Buttons};

/// Main loop period, also the buttons debounce sample period.
const TICK_MS: u16 = 1;

/// ADC samples averaged per axis reading.
const OVERSAMPLE: u32 = 4;

/// Samples averaged to find the stick center at power on.
const CALIBRATION_SAMPLES: u32 = 16;

#[entry]
fn main() -> ! {
    let mut p = pac::Peripherals::take().unwrap();
    let cp = Peripherals::take().unwrap();

    let mut rcc = p.RCC.configure().sysclk(8.mhz()).freeze(&mut p.FLASH);

    let mut watchdog = Watchdog::new(p.IWDG);
    watchdog.start(10.hz());

    // Release INT before the pin becomes an output, so it never glitches low.
    i2c_slave::set_int(false);

    let gpioa = p.GPIOA.split(&mut rcc);
    let (mut stick_power, mut x_pin, mut y_pin, stick_pin, left_pin, right_pin) =
        cortex_m::interrupt::free(|cs| {
            // Only configured, driven through BSRR by `i2c_slave`.
            gpioa.pa6.into_open_drain_output(cs);
            gpioa.pa9.into_alternate_af4(cs).set_open_drain(cs);
            gpioa.pa10.into_alternate_af4(cs).set_open_drain(cs);
            (
                gpioa.pa0.into_push_pull_output(cs),
                gpioa.pa1.into_analog(cs),
                gpioa.pa2.into_analog(cs),
                gpioa.pa3.into_pull_up_input(cs),
                gpioa.pa4.into_pull_up_input(cs),
                gpioa.pa5.into_pull_up_input(cs),
            )
        });

    let mut delay = Delay::new(cp.SYST, &rcc);
    let mut adc = Adc::new(p.ADC, &mut rcc);

    stick_power.set_high().ok();
    delay.delay_ms(10_u16);

    let x_axis = Axis::new(average(&mut adc, &mut x_pin, CALIBRATION_SAMPLES));
    let y_axis = Axis::new(average(&mut adc, &mut y_pin, CALIBRATION_SAMPLES));
    let mut buttons = Buttons::new();

    i2c_slave::init(p.I2C1);

    loop {
        let raw_buttons = (stick_pin.is_low().unwrap_or(false) as u8) << BUTTON_STICK
            | (left_pin.is_low().unwrap_or(false) as u8) << BUTTON_LEFT
            | (right_pin.is_low().unwrap_or(false) as u8) << BUTTON_RIGHT;

        i2c_slave::publish(State {
            x: x_axis.value(average(&mut adc, &mut x_pin, OVERSAMPLE)),
            y: y_axis.value(average(&mut adc, &mut y_pin, OVERSAMPLE)),
            buttons: buttons.update(raw_buttons),
        });

        watchdog.feed();
        delay.delay_ms(TICK_MS);
    }
}

fn average<P: Channel<Adc, ID = u8>>(adc: &mut Adc, pin: &mut P, samples: u32) -> u16 {
    let sum: u32 = (0..samples).map(|_| adc.read(pin).unwrap_or(0_u16) as u32).sum();
    (sum / samples) as u16
}
