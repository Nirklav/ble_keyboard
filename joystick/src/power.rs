//! Low power: the main loop sleeps (WFI) between its 1 ms ticks ([`Ticker`]), and the whole MCU stops
//! while idle ([`stop`]). Stop is left on:
//! - the RTC alarm every 25 ms: a deflection of the stick can't wake it, so it's sampled then;
//! - a press, a falling edge on PA3/PA4/PA5;
//! - I2C traffic, a falling edge on SDA (PA10): I2C1 of the F030 can't wake from Stop on its address
//!   (DS9773 Table 7), so the keyboard's traffic before the joystick is read (the display at boot) wakes it.
//!
//! The IWDG runs on in Stop and can't be frozen on the F030, so it's fed on every RTC tick: both count the
//! LSI, whatever its frequency (30..50 kHz) the tick stays shorter than the watchdog period.

use core::cell::Cell;

use cortex_m::interrupt::{CriticalSection, Mutex};
use cortex_m::peripheral::syst::SystClkSource;
use cortex_m::peripheral::{NVIC, SCB, SYST};

use crate::hal::pac::{self, interrupt, EXTI, I2C1, PWR, RCC, RTC};

/// SysTick reload for 1 ms at HCLK = 8 MHz (HSI).
const SYSTICK_RELOAD: u32 = 8_000 - 1;

/// The RTC "second" is the Stop tick: (PREDIV_A + 1) * (PREDIV_S + 1) / LSI = 125 * 8 / 40 kHz = 25 ms.
/// PREDIV_S must be 3 at least (RM0360 21.4.5).
const RTC_PREDIV_A: u8 = 124;
const RTC_PREDIV_S: u16 = 7;

/// What woke the MCU from [`stop`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The RTC tick, or nothing armed (another interrupt).
    Tick,
    /// A button or the stick press.
    Press,
    /// I2C traffic.
    Bus,
}

const WAKE_PRESS: u8 = 1 << 0;
const WAKE_BUS: u8 = 1 << 1;

static TICK: Mutex<Cell<bool>> = Mutex::new(Cell::new(false));
static WOKEN_BY: Mutex<Cell<u8>> = Mutex::new(Cell::new(0));

/// The 1 ms tick of the main loop, the core sleeps until it.
pub struct Ticker {
    _syst: SYST,
}

impl Ticker {
    pub fn new(mut syst: SYST) -> Self {
        syst.set_clock_source(SystClkSource::Core);
        syst.set_reload(SYSTICK_RELOAD);
        syst.clear_current();
        syst.enable_interrupt();
        syst.enable_counter();
        Self { _syst: syst }
    }

    /// Sleeps until the next tick. Interrupts are masked from the check to WFI, so a tick in between isn't
    /// missed: a pending interrupt wakes WFI with PRIMASK set too, its handler runs after.
    pub fn wait(&mut self) {
        loop {
            let ticked = cortex_m::interrupt::free(|cs| {
                if TICK.borrow(cs).replace(false) {
                    return true;
                }
                cortex_m::asm::wfi();
                false
            });
            if ticked {
                return;
            }
        }
    }
}

#[cortex_m_rt::exception]
fn SysTick() {
    cortex_m::interrupt::free(|cs| TICK.borrow(cs).set(true));
}

/// The RTC alarm on every Stop tick and the EXTI lines of the wake-ups, armed only in [`stop`].
pub fn init(rtc: RTC, exti: &EXTI, pwr: &PWR) {
    // The HAL `Rcc` keeps the registers private, these bits are not touched by it after `freeze`.
    let rcc = unsafe { &*RCC::ptr() };
    rcc.apb1enr.modify(|_, w| w.pwren().set_bit());

    // The LSI runs for the IWDG already. The RTC domain survives a reset: it starts over from a known state.
    rcc.csr.modify(|_, w| w.lsion().set_bit());
    while rcc.csr.read().lsirdy().bit_is_clear() {}
    pwr.cr.modify(|_, w| w.dbp().set_bit());
    rcc.bdcr.modify(|_, w| w.bdrst().set_bit());
    rcc.bdcr.modify(|_, w| w.bdrst().clear_bit());
    rcc.bdcr.modify(|_, w| w.rtcsel().lsi().rtcen().set_bit());

    rtc.wpr.write(|w| unsafe { w.key().bits(0xCA) });
    rtc.wpr.write(|w| unsafe { w.key().bits(0x53) });
    rtc.isr.modify(|_, w| w.init().set_bit());
    while rtc.isr.read().initf().bit_is_clear() {}
    // Two separate writes (RM0360 21.4.3).
    rtc.prer.modify(|_, w| unsafe { w.prediv_s().bits(RTC_PREDIV_S) });
    rtc.prer.modify(|_, w| unsafe { w.prediv_a().bits(RTC_PREDIV_A) });
    rtc.cr.modify(|_, w| w.alrae().clear_bit());
    while rtc.isr.read().alrawf().bit_is_clear() {}
    // Every field masked, no sub-seconds compared: the alarm comes every "second", every tick.
    rtc.alrmar.write(|w| w.msk1().set_bit().msk2().set_bit().msk3().set_bit().msk4().set_bit());
    rtc.alrmassr.write(|w| unsafe { w.bits(0) });
    rtc.cr.modify(|_, w| w.alraie().set_bit().alrae().set_bit());
    rtc.isr.modify(|_, w| w.init().clear_bit());
    rtc.wpr.write(|w| unsafe { w.key().bits(0xFF) });

    // EXTI 17 is the RTC alarm (RM0360 11.2.5), on its rising edge; an alarm while running is harmless.
    exti.rtsr.modify(|_, w| w.tr17().set_bit());
    exti.imr.modify(|_, w| w.mr17().set_bit());
    // The presses (active low) and SDA, port A is the reset value of SYSCFG_EXTICR.
    exti.ftsr.modify(|_, w| w.tr3().set_bit().tr4().set_bit().tr5().set_bit().tr10().set_bit());

    unsafe {
        NVIC::unmask(pac::Interrupt::RTC);
        NVIC::unmask(pac::Interrupt::EXTI2_3);
        NVIC::unmask(pac::Interrupt::EXTI4_15);
    }
}

/// Stops the MCU (low-power regulator) until a wake-up. Only when idle: nothing pressed, the stick centered,
/// `INT` released. `still_idle` checks the pins again once the edges are armed, a press in between isn't lost.
pub fn stop(scb: &mut SCB, still_idle: impl Fn() -> bool) -> Wake {
    let exti = unsafe { &*EXTI::ptr() };
    let pwr = unsafe { &*PWR::ptr() };
    let i2c = unsafe { &*I2C1::ptr() };

    cortex_m::interrupt::free(|cs| WOKEN_BY.borrow(cs).set(0));
    // SDA toggles during every transaction while running: stale edges are dropped before arming.
    exti.pr.write(|w| w.pr3().set_bit().pr4().set_bit().pr5().set_bit().pr10().set_bit());
    exti.imr.modify(|_, w| w.mr3().set_bit().mr4().set_bit().mr5().set_bit().mr10().set_bit());

    cortex_m::interrupt::free(|_| {
        if !still_idle() || i2c.isr.read().busy().bit_is_set() {
            return;
        }
        pwr.cr.modify(|_, w| w.pdds().clear_bit().lpds().set_bit());
        scb.set_sleepdeep();
        // Returns at once if an armed line is pending already.
        cortex_m::asm::wfi();
        scb.clear_sleepdeep();
    });
    // The handlers have run. SYSCLK is the HSI at 8 MHz again (DS9773 6.3.6), as configured.

    exti.imr.modify(|_, w| w.mr3().clear_bit().mr4().clear_bit().mr5().clear_bit().mr10().clear_bit());
    // The ADC clock (HSI14) stopped with the other oscillators.
    let rcc = unsafe { &*RCC::ptr() };
    rcc.cr2.modify(|_, w| w.hsi14on().set_bit());
    while rcc.cr2.read().hsi14rdy().bit_is_clear() {}

    let woken_by = cortex_m::interrupt::free(|cs| WOKEN_BY.borrow(cs).get());
    if woken_by & WAKE_PRESS != 0 {
        Wake::Press
    } else if woken_by & WAKE_BUS != 0 {
        Wake::Bus
    } else {
        Wake::Tick
    }
}

fn woken(cs: &CriticalSection, cause: u8) {
    let woken_by = WOKEN_BY.borrow(cs);
    woken_by.set(woken_by.get() | cause);
}

#[interrupt]
fn RTC() {
    let rtc = unsafe { &*RTC::ptr() };
    let exti = unsafe { &*EXTI::ptr() };
    rtc.isr.modify(|_, w| w.alraf().clear_bit());
    exti.pr.write(|w| w.pr17().set_bit());
}

#[interrupt]
fn EXTI2_3() {
    let exti = unsafe { &*EXTI::ptr() };
    exti.pr.write(|w| w.pr3().set_bit());
    cortex_m::interrupt::free(|cs| woken(cs, WAKE_PRESS));
}

#[interrupt]
fn EXTI4_15() {
    let exti = unsafe { &*EXTI::ptr() };
    let pr = exti.pr.read();
    let mut cause = 0;
    if pr.pr4().bit_is_set() || pr.pr5().bit_is_set() {
        cause |= WAKE_PRESS;
    }
    if pr.pr10().bit_is_set() {
        cause |= WAKE_BUS;
    }
    exti.pr.write(|w| w.pr4().set_bit().pr5().set_bit().pr10().set_bit());
    cortex_m::interrupt::free(|cs| woken(cs, cause));
}
