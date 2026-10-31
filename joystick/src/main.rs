//! Joystick extension: an analog stick with a press and two buttons, read by the
//! keyboard over I2C (see [`protocol::joystick`]).
//!
//! Pins (see `hardware/schematic.pdf`):
//! - PA0: stick potentiometers supply, on only while they are sampled
//! - PA1, PA2: stick X, Y (ADC)
//! - PA3: stick press, PA4: left button, PA5: right button (to GND)
//! - PA6: `INT` to the keyboard
//! - PA9, PA10: I2C1 SCL, SDA (pull-ups are on the keyboard)
//!
//! Sleeps between the ticks and stops while idle, see [`power`].
#![no_main]
#![no_std]

mod i2c_slave;
mod input;
mod power;

use panic_halt as _;
use stm32f0xx_hal as hal;

use cortex_m::peripheral::Peripherals;
use cortex_m_rt::entry;
use embedded_hal::adc::{Channel, OneShot};

use crate::hal::adc::{Adc, AdcSampleTime};
use crate::hal::watchdog::Watchdog;
use crate::hal::{pac, prelude::*};

use protocol::joystick::{State, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_STICK};

use crate::input::{Axis, Buttons};
use crate::power::Wake;

/// ADC samples averaged per axis reading.
const OVERSAMPLE: u32 = 4;

/// Samples averaged to find the stick center at power on.
const CALIBRATION_SAMPLES: u32 = 16;

/// From powering the pots to sampling: they settle in well under 1 µs (a wiper of 2.5 kΩ at most into
/// ~20 pF, no capacitors on them); 10 µs at 8 MHz.
const POT_SETTLE_CYCLES: u32 = 80;

/// Idle this long (the stick centered, nothing pressed, `INT` released) before it stops, in ticks of 1 ms.
const IDLE_TICKS: u32 = 500;

/// Awake this long after I2C traffic woke it: the keyboard reads the version right after its display init.
const BUS_WAKE_TICKS: u32 = 200;

#[entry]
fn main() -> ! {
    let mut p = pac::Peripherals::take().unwrap();
    let mut cp = Peripherals::take().unwrap();

    let mut rcc = p.RCC.configure().sysclk(8.mhz()).freeze(&mut p.FLASH);

    let mut watchdog = Watchdog::new(p.IWDG);
    // 250 ms: ten Stop ticks, both count the LSI.
    watchdog.start(4.hz());

    // Release INT before the pin becomes an output, so it never glitches low.
    i2c_slave::set_int(false);

    let gpioa = p.GPIOA.split(&mut rcc);
    let gpiob = p.GPIOB.split(&mut rcc);
    let gpiof = p.GPIOF.split(&mut rcc);
    let (mut stick_power, mut x_pin, mut y_pin, stick_pin, left_pin, right_pin) =
        cortex_m::interrupt::free(|cs| {
            // Only configured, driven through BSRR by `i2c_slave`.
            gpioa.pa6.into_open_drain_output(cs);
            gpioa.pa9.into_alternate_af4(cs).set_open_drain(cs);
            gpioa.pa10.into_alternate_af4(cs).set_open_drain(cs);
            // Not connected: analog, a floating input draws current in Stop. PA13/PA14 (SWD) are pulled.
            gpioa.pa7.into_analog(cs);
            gpiob.pb1.into_analog(cs);
            gpiof.pf0.into_analog(cs);
            gpiof.pf1.into_analog(cs);
            (
                gpioa.pa0.into_push_pull_output(cs),
                gpioa.pa1.into_analog(cs),
                gpioa.pa2.into_analog(cs),
                gpioa.pa3.into_pull_up_input(cs),
                gpioa.pa4.into_pull_up_input(cs),
                gpioa.pa5.into_pull_up_input(cs),
            )
        });

    let mut adc = Adc::new(p.ADC, &mut rcc);
    // 41.5 cycles allow up to 37 kΩ at the input (DS9773 Table 51), the wiper is 2.5 kΩ at most;
    // a conversion takes 3.9 µs instead of 18 µs at the default 239.5.
    adc.set_sample_time(AdcSampleTime::T_41);

    let mut ticker = power::Ticker::new(cp.SYST);
    power::init(p.RTC, &p.EXTI, &p.PWR);

    // The pots stay powered for the whole calibration.
    stick_power.set_high().ok();
    cortex_m::asm::delay(POT_SETTLE_CYCLES);
    let x_axis = Axis::new(average(&mut adc, &mut x_pin, CALIBRATION_SAMPLES));
    let y_axis = Axis::new(average(&mut adc, &mut y_pin, CALIBRATION_SAMPLES));
    stick_power.set_low().ok();
    let mut buttons = Buttons::new();

    i2c_slave::init(p.I2C1);

    let raw_buttons = || {
        (stick_pin.is_low().unwrap_or(false) as u8) << BUTTON_STICK
            | (left_pin.is_low().unwrap_or(false) as u8) << BUTTON_LEFT
            | (right_pin.is_low().unwrap_or(false) as u8) << BUTTON_RIGHT
    };

    // Ticks left until it stops, counted down while idle.
    let mut awake = IDLE_TICKS;
    // Just woken by the RTC tick: sampled at once, not a tick later.
    let mut sample_now = false;

    loop {
        if !sample_now {
            ticker.wait();
        }
        sample_now = false;

        let raw = raw_buttons();
        stick_power.set_high().ok();
        cortex_m::asm::delay(POT_SETTLE_CYCLES);
        let x = average(&mut adc, &mut x_pin, OVERSAMPLE);
        let y = average(&mut adc, &mut y_pin, OVERSAMPLE);
        stick_power.set_low().ok();

        let state = State { x: x_axis.value(x), y: y_axis.value(y), buttons: buttons.update(raw) };
        i2c_slave::publish(state);
        watchdog.feed();

        // A press still being debounced, or a state the keyboard hasn't read yet, keeps it awake.
        if state.is_active() || raw != 0 || i2c_slave::int_asserted() {
            awake = awake.max(IDLE_TICKS);
            continue;
        }
        awake = awake.saturating_sub(1);
        if awake > 0 {
            continue;
        }

        let wake = power::stop(&mut cp.SCB, || raw_buttons() == 0 && !i2c_slave::int_asserted());
        watchdog.feed();
        match wake {
            // Debounced on the ticks as usual.
            Wake::Press => awake = IDLE_TICKS,
            Wake::Bus => awake = BUS_WAKE_TICKS,
            // Sampled once, back to Stop if still idle.
            Wake::Tick => {
                awake = 1;
                sample_now = true;
            }
        }
    }
}

fn average<P: Channel<Adc, ID = u8>>(adc: &mut Adc, pin: &mut P, samples: u32) -> u16 {
    let sum: u32 = (0..samples).map(|_| adc.read(pin).unwrap_or(0_u16) as u32).sum();
    (sum / samples) as u16
}
