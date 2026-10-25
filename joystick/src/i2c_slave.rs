//! I2C1 slave serving [`protocol::joystick`] registers, and the `INT` line.
//!
//! The HAL only implements the master mode, so the peripheral is driven through the PAC.
//! All bus events are handled in the `I2C1` interrupt, the main loop only publishes
//! the latest [`State`] with [`publish`].

use core::cell::{Cell, RefCell};

use cortex_m::interrupt::{CriticalSection, Mutex};
use protocol::joystick::{Register, State, ADDRESS, VERSION};

use crate::hal::pac::{self, interrupt, GPIOA, I2C1, RCC};

/// `INT` pin: PA6, open-drain, active low.
const INT_PIN: u8 = 6;

/// Slave timings for an 8 MHz I2C clock (HSI), from the reference manual: SDADEL = 1,
/// SCLDEL = 3. Suits both 100 kHz and 400 kHz masters, SCLL/SCLH are unused by a slave.
const TIMINGR: u32 = 0x0031_0309;

/// Value returned for bytes read past the end of a register.
const FILLER: u8 = 0xFF;

/// Latest state, written by the main loop.
static CURRENT: Mutex<Cell<State>> = Mutex::new(Cell::new(State { x: 0, y: 0, buttons: 0 }));
/// Last state completely read by the master.
static REPORTED: Mutex<Cell<State>> = Mutex::new(Cell::new(State { x: 0, y: 0, buttons: 0 }));

/// Owned by the interrupt handler.
struct Transaction {
    /// Register selected by the last write.
    register: u8,
    /// Next received byte is the register address.
    expect_register: bool,
    tx: [u8; State::SIZE],
    tx_len: usize,
    tx_pos: usize,
    /// State being read by the master, becomes reported on STOP.
    sending: Option<State>,
}

static TRANSACTION: Mutex<RefCell<Transaction>> = Mutex::new(RefCell::new(Transaction {
    register: Register::State as u8,
    expect_register: false,
    tx: [0; State::SIZE],
    tx_len: 0,
    tx_pos: 0,
    sending: None,
}));

/// Enables I2C1 as a slave at [`ADDRESS`] and its interrupt.
/// SCL/SDA (PA9/PA10) must already be in AF4 open-drain mode.
pub fn init(i2c: I2C1) {
    // The HAL `Rcc` keeps the registers private, these bits are not touched by it after `freeze`.
    let rcc = unsafe { &*RCC::ptr() };
    rcc.apb1enr.modify(|_, w| w.i2c1en().set_bit());
    rcc.apb1rstr.modify(|_, w| w.i2c1rst().set_bit());
    rcc.apb1rstr.modify(|_, w| w.i2c1rst().clear_bit());

    i2c.timingr.write(|w| unsafe { w.bits(TIMINGR) });
    i2c.oar1.write(|w| w.oa1().bits((ADDRESS as u16) << 1).oa1en().set_bit());
    i2c.cr1.write(|w| {
        w.addrie().set_bit()
            .rxie().set_bit()
            .txie().set_bit()
            .stopie().set_bit()
            .nackie().set_bit()
            .errie().set_bit()
            .pe().set_bit()
    });

    unsafe { cortex_m::peripheral::NVIC::unmask(pac::Interrupt::I2C1) };
}

/// Publishes the latest state and updates `INT`.
pub fn publish(state: State) {
    cortex_m::interrupt::free(|cs| {
        CURRENT.borrow(cs).set(state);
        update_int(cs);
    });
}

fn update_int(cs: &CriticalSection) {
    let current = CURRENT.borrow(cs).get();
    let pending = current.is_active() || current != REPORTED.borrow(cs).get();
    set_int(pending);
}

/// Drives `INT` low when `asserted`, releases it otherwise.
/// BSRR writes are atomic, so it's safe from both the main loop and the interrupt.
pub fn set_int(asserted: bool) {
    let gpioa = unsafe { &*GPIOA::ptr() };
    if asserted {
        gpioa.bsrr.write(|w| unsafe { w.bits(1 << (INT_PIN + 16)) });
    } else {
        gpioa.bsrr.write(|w| unsafe { w.bits(1 << INT_PIN) });
    }
}

#[interrupt]
fn I2C1() {
    let i2c = unsafe { &*I2C1::ptr() };

    cortex_m::interrupt::free(|cs| {
        let mut t = TRANSACTION.borrow(cs).borrow_mut();
        let isr = i2c.isr.read();

        // Before ADDR: the register byte may still be pending when a repeated start arrives.
        if isr.rxne().bit_is_set() {
            let byte = i2c.rxdr.read().rxdata().bits();
            if t.expect_register {
                t.register = byte;
                t.expect_register = false;
            }
        }

        if isr.addr().bit_is_set() {
            if isr.dir().bit_is_set() {
                // Master reads: drop a byte left in TXDR by the previous transaction.
                i2c.isr.write(|w| w.txe().set_bit());
                t.tx_pos = 0;
                t.sending = None;
                match Register::try_from(t.register) {
                    Ok(Register::Version) => {
                        t.tx[0] = VERSION;
                        t.tx_len = Register::Version.size();
                    }
                    Ok(Register::State) => {
                        let state = CURRENT.borrow(cs).get();
                        t.tx = state.encode();
                        t.tx_len = Register::State.size();
                        t.sending = Some(state);
                    }
                    Err(_) => t.tx_len = 0,
                }
            } else {
                t.expect_register = true;
            }
            i2c.icr.write(|w| w.addrcf().set_bit());
        }

        if isr.txis().bit_is_set() {
            let byte = if t.tx_pos < t.tx_len { t.tx[t.tx_pos] } else { FILLER };
            t.tx_pos += 1;
            i2c.txdr.write(|w| w.txdata().bits(byte));
        }

        if isr.nackf().bit_is_set() {
            // The master NACKs the last byte it wants.
            i2c.icr.write(|w| w.nackcf().set_bit());
        }

        if isr.berr().bit_is_set() || isr.arlo().bit_is_set() || isr.ovr().bit_is_set() {
            i2c.icr.write(|w| w.berrcf().set_bit().arlocf().set_bit().ovrcf().set_bit());
            t.sending = None;
        }

        if isr.stopf().bit_is_set() {
            i2c.icr.write(|w| w.stopcf().set_bit());
            t.expect_register = false;
            // TXDR is preloaded ahead of the bus, so `tx_pos` may exceed the bytes actually
            // sent by one; a partial read of the state is a protocol violation anyway.
            if let Some(state) = t.sending.take() {
                if t.tx_pos >= t.tx_len {
                    REPORTED.borrow(cs).set(state);
                    update_int(cs);
                }
            }
        }
    });
}
