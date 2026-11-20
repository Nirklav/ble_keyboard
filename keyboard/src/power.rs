//! Sleep: the main loop waits for the key interrupts, idles the BLE link after [`IDLE_DELAY`] and deep sleeps
//! after the configured time (see [`protocol::config::Settings`]). Every wake-up source is an interrupt pin, active low.

use std::ffi::c_void;
use std::num::NonZeroU32;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{Input, InterruptType, PinDriver};
use esp_idf_svc::hal::interrupt;
use esp_idf_svc::hal::task::notification::Notification;
use esp_idf_sys as sys;
use log::error;
use protocol::config::Settings;

use crate::display::Display;
use crate::hardware::Matrix;
use crate::joystick::Joystick;
use crate::sleep_admission::{IdlePeriod, SLEEP};

/// Wakes the main loop to process keys, configuration or firmware updates.
pub(crate) const ACTIVITY: NonZeroU32 = NonZeroU32::new(1).unwrap();
/// Rechecks the sleep deadline when the BLE host changes.
pub(crate) const HOST_CHANGED: NonZeroU32 = NonZeroU32::new(2).unwrap();

/// The loop goes idle after this long without activity, see [`Settings::SLEEP_S`].
pub(crate) const IDLE_DELAY: Duration = Duration::from_secs(15);
// The deep sleep timeouts are counted from the last activity, so they can't be shorter than the idle delay.
const _: () = assert!(IDLE_DELAY.as_secs() <= *Settings::SLEEP_S.start() as u64);

const DISPLAY_OFF_MAX_ATTEMPTS: u8 = 2;
const PREPARATION_RETRY: Duration = Duration::from_secs(5);

// Set before notifying the main task. The GPIO ISR can run during any pre-commit preparation.
static ACTIVITY_LATCHED: AtomicBool = AtomicBool::new(false);

pub(crate) fn latch_activity() {
    ACTIVITY_LATCHED.store(true, Ordering::Relaxed);
}

fn wait_until(notification: &Notification, deadline: Instant) -> Option<NonZeroU32> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let events = notification.wait(TickType::from(remaining).into());
        // A tick timeout can finish just before the deadline.
        if events.is_some() || Instant::now() >= deadline {
            return events;
        }
    }
}

pub(crate) fn activity_before(notification: &Notification, deadline: Instant) -> bool {
    while let Some(events) = wait_until(notification, deadline) {
        if events.get() & ACTIVITY.get() != 0 {
            return true;
        }
    }
    false
}

pub(crate) fn arm_activity_interrupts(
    key_matrix: &mut Matrix,
    joystick: &mut Option<Joystick>,
) -> anyhow::Result<()> {
    ACTIVITY_LATCHED.store(false, Ordering::Relaxed);
    key_matrix.irq_mut().enable_interrupt()?;
    if let Some(joystick) = joystick {
        joystick.irq_mut().enable_interrupt()?;
    }
    Ok(())
}

/// While the radio uses idle connection parameters, wait for activity or transition to deep sleep.
pub(crate) fn wait_in_radio_idle(
    notification: &Notification,
    display: &Display,
    irq_pins: &[&PinDriver<'static, Input>],
    idle: IdlePeriod,
) {
    let mut check_at = Instant::now();
    loop {
        if activity_pending(irq_pins) {
            break;
        }
        if let Some(events) = wait_until(notification, check_at) {
            if events.get() & ACTIVITY.get() != 0 {
                break;
            }
            // A host change can shorten the deadline without starting another idle period.
            check_at = Instant::now();
            continue;
        }
        match attempt_deep_sleep(display, irq_pins, idle) {
            SleepWait::Activity => break,
            SleepWait::RetryAt(at) => check_at = at,
        }
    }
}

enum SleepWait {
    Activity,
    RetryAt(Instant),
}

fn activity_pending(irq_pins: &[&PinDriver<'static, Input>]) -> bool {
    ACTIVITY_LATCHED.load(Ordering::Relaxed) || irq_pins.iter().any(|pin| pin.is_low())
}

fn check_preparation(irq_pins: &[&PinDriver<'static, Input>], idle: IdlePeriod) -> Result<(), SleepWait> {
    if activity_pending(irq_pins) {
        return Err(SleepWait::Activity);
    }
    SLEEP.check(idle, Instant::now()).map_err(SleepWait::RetryAt)
}

fn attempt_deep_sleep(display: &Display, irq_pins: &[&PinDriver<'static, Input>], idle: IdlePeriod) -> SleepWait {
    if let Err(wait) = check_preparation(irq_pins, idle) {
        return wait;
    }

    for attempt in 1..=DISPLAY_OFF_MAX_ATTEMPTS {
        match display.prepare_sleep() {
            Ok(()) => break,
            Err(e) => error!("display off before deep sleep (attempt {}): {:?}", attempt, e),
        }
        if attempt < DISPLAY_OFF_MAX_ATTEMPTS {
            if let Err(wait) = check_preparation(irq_pins, idle) {
                display.cancel_sleep();
                return wait;
            }
        }
    }

    let wait = match configure_deep_sleep(irq_pins) {
        Ok(()) => try_enter_deep_sleep(irq_pins, idle),
        Err(e) => {
            error!("preparing deep sleep: {:?}", e);
            SleepWait::RetryAt(Instant::now() + PREPARATION_RETRY)
        },
    };
    cancel_deep_sleep(irq_pins);
    display.cancel_sleep();
    wait
}

fn try_enter_deep_sleep(irq_pins: &[&PinDriver<'static, Input>], idle: IdlePeriod) -> SleepWait {
    let mut commit = match SLEEP.prepare_commit(idle, Instant::now()) {
        Ok(commit) => commit,
        Err(at) => return SleepWait::RetryAt(at),
    };
    // On the C3, exclude GPIO ISRs for the last latch/level check and publication.
    // The armed RTC GPIOs handle a later assertion.
    let committed = interrupt::free(|| {
        if activity_pending(irq_pins) {
            return false;
        }
        commit.publish();
        true
    });
    drop(commit);
    if !committed {
        return SleepWait::Activity;
    }
    // ESP-IDF may wait on a semaphore. Enter with the application gate and IRQ mask released.
    unsafe { sys::esp_deep_sleep_start() }
}

pub fn woke_from_deep_sleep() -> bool {
    unsafe { esp_idf_sys::esp_reset_reason() == esp_idf_sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP }
}

/// Interrupt pins that wake the keyboard up, all active low.
pub fn wakeup_pins<'a>(
    key_matrix: &'a Matrix,
    joystick: &'a Option<Joystick>,
) -> Vec<&'a PinDriver<'static, Input>> {
    let mut pins = vec![key_matrix.irq()];
    if let Some(joystick) = joystick {
        pins.push(joystick.irq());
    }
    pins
}

pub fn prepare_light_sleep(irq_pins: &[&PinDriver<'static, Input>]) -> anyhow::Result<()> {
    unsafe {
        for irq_pin in irq_pins {
            sys::esp!(sys::gpio_wakeup_enable(irq_pin.pin().into(), sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL))?;
        }
        sys::esp!(sys::esp_sleep_enable_gpio_wakeup())?;
    }
    Ok(())
}

pub fn release_deep_sleep_holds(irq_pins: &[&PinDriver<'static, Input>]) -> anyhow::Result<()> {
    unsafe {
        sys::gpio_deep_sleep_hold_dis();
        for irq_pin in irq_pins {
            sys::esp!(sys::gpio_hold_dis(irq_pin.pin().into()))?;
        }
    }
    Ok(())
}

/// Automatic light sleep while every task waits, the BLE connection stays (see sdkconfig.defaults:
/// the main crystal stays powered for the BT controller). Before the BLE stack starts.
pub fn enable_light_sleep() -> anyhow::Result<()> {
    configure_pm(true)
}

/// The CPU between 40 and 80 MHz, light sleep when idle if `light_sleep`.
fn configure_pm(light_sleep: bool) -> anyhow::Result<()> {
    let config = esp_idf_sys::esp_pm_config_t { max_freq_mhz: 80, min_freq_mhz: 40, light_sleep_enable: light_sleep };
    esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_pm_configure(&config as *const _ as *const c_void) })?;
    Ok(())
}

/// Keeps the chip out of light sleep while held, e.g. while the main loop polls held keys: the interrupt
/// line stays low then, every attempt to sleep would be rejected only after preparing it.
pub struct NoLightSleep {
    lock: esp_idf_sys::esp_pm_lock_handle_t,
    held: bool,
}

impl NoLightSleep {
    pub fn new() -> anyhow::Result<Self> {
        let mut lock = ptr::null_mut();
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_pm_lock_create(esp_idf_sys::esp_pm_lock_type_t_ESP_PM_NO_LIGHT_SLEEP, 0, c"keys".as_ptr(), &mut lock)
        })?;
        Ok(Self { lock, held: false })
    }

    pub fn hold(&mut self, hold: bool) {
        if hold == self.held {
            return;
        }
        unsafe {
            if hold {
                esp_idf_sys::esp_pm_lock_acquire(self.lock);
            } else {
                esp_idf_sys::esp_pm_lock_release(self.lock);
            }
        }
        self.held = hold;
    }
}

fn configure_deep_sleep(irq_pins: &[&PinDriver<'static, Input>]) -> anyhow::Result<()> {
    // Auto light sleep leaves its timer wake-up source on: deep sleep would wake up (restart) at once.
    configure_pm(false).context("turn off automatic light sleep")?;
    unsafe {
        let mut mask = 0_u64;
        for irq_pin in irq_pins {
            sys::esp!(sys::gpio_hold_en(irq_pin.pin().into())).with_context(|| format!("hold GPIO {}", irq_pin.pin()))?;
            mask |= 1 << irq_pin.pin();
        }
        sys::gpio_deep_sleep_hold_en();
        sys::esp!(sys::esp_deep_sleep_enable_gpio_wakeup(
            mask,
            sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_LOW,
        )).context("arm deep-sleep GPIO wake")?;
    }
    Ok(())
}

fn cancel_deep_sleep(irq_pins: &[&PinDriver<'static, Input>]) {
    unsafe {
        sys::gpio_deep_sleep_hold_dis();
        for irq_pin in irq_pins {
            let pin = i32::from(irq_pin.pin());
            if let Err(e) = sys::esp!(sys::gpio_hold_dis(pin)) {
                error!("release deep-sleep hold for GPIO {}: {:?}", pin, e);
            }
            if let Err(e) = sys::esp!(sys::gpio_deep_sleep_wakeup_disable(pin)) {
                error!("disarm deep-sleep wake for GPIO {}: {:?}", pin, e);
            }
        }
    }
    if let Err(e) = prepare_light_sleep(irq_pins) {
        error!("restore light-sleep GPIO wake: {:?}", e);
    }
    if let Err(e) = enable_light_sleep() {
        error!("restore automatic light sleep: {:?}", e);
    }
}

/// `ext_irq` going low wakes the main loop up through `notification`.
pub fn setup_interrupt(ext_irq: &mut PinDriver<'static, Input>, notification: Rc<Notification>) -> anyhow::Result<()> {
    ext_irq.set_interrupt_type(InterruptType::LowLevel)?;

    let notifier = notification.notifier();
    unsafe {
        ext_irq.subscribe(move || {
            latch_activity();
            notifier.notify_and_yield(ACTIVITY);
        })?;
    }
    Ok(())
}
