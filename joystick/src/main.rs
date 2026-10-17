#![no_main]
#![no_std]

use panic_halt as _;
use stm32f0xx_hal::gpio::gpioa::{PA10, PA9};
use stm32f0xx_hal::gpio::{Alternate, AF4};
use stm32f0xx_hal::pac::I2C1;
use stm32f0xx_hal as hal;

use shared_bus::{BusManagerSimple, I2cProxy, NullMutex};

use crate::hal::{pac, prelude::*};
use crate::hal::delay::Delay;
use crate::hal::i2c::I2c;

use heapless::String;

use cortex_m::peripheral::Peripherals;
use cortex_m_rt::entry;

#[entry]
fn main() -> ! {
    if let (Some(mut p), Some(cp)) = (pac::Peripherals::take(), Peripherals::take()) {
        let mut rcc = p.RCC.configure()
            .sysclk(8.mhz())
            .freeze(&mut p.FLASH);

        let gpioa = p.GPIOA.split(&mut rcc);

        let (mut led_pin, scl_pin, sda_pin) = cortex_m::interrupt::free(move |cs| {
            (
                gpioa.pa2.into_push_pull_output(cs),
                gpioa.pa9.into_alternate_af4(cs), //scl
                gpioa.pa10.into_alternate_af4(cs) //sda
            )
        });
        
        led_pin.set_high().ok();

        let mut delay = Delay::new(cp.SYST, &rcc);

        let bus = BusManagerSimple::new(I2c::i2c1(p.I2C1, (scl_pin, sda_pin), 100.khz(), &mut rcc));

        let mut buffer = String::<16>::new();

        led_pin.set_low().ok();
        loop {

            delay.delay_ms(1_000_u32); // 1 sec
        }
    }

    loop {
        continue;
    }
}