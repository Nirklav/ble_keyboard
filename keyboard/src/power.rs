//! Sleep: the main loop waits for the key interrupts, idles the BLE link after [`IDLE_DELAY`] and deep sleeps
//! after the configured time (see [`protocol::config::Settings`]). Every wake-up source is an interrupt pin, active low.

use std::ffi::c_void;
use std::num::NonZeroU32;
use std::ptr;
use std::rc::Rc;
use std::time::{Duration, Instant};

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{Input, InterruptType, PinDriver};
use esp_idf_svc::hal::task::notification::Notification;
use log::{error, info};
use protocol::config::Settings;

use crate::firmware;
use crate::hardware::Matrix;
use crate::joystick::Joystick;
use crate::services::Services;

/// Wakes the main loop to process keys, configuration or firmware updates.
pub(crate) const ACTIVITY: NonZeroU32 = NonZeroU32::new(1).unwrap();
/// Rechecks the sleep deadline when the BLE host changes.
pub(crate) const HOST_CHANGED: NonZeroU32 = NonZeroU32::new(2).unwrap();

/// The loop goes idle after this long without activity, see [`Settings::SLEEP_S`].
const IDLE_DELAY: Duration = Duration::from_secs(15);
// The deep sleep timeouts are counted from the last activity, so they can't be shorter than the idle delay.
const _: () = assert!(IDLE_DELAY.as_secs() <= *Settings::SLEEP_S.start() as u64);

const DISPLAY_OFF_MAX_ATTEMPTS: u8 = 2;

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

fn activity_before(notification: &Notification, deadline: Instant) -> bool {
    while let Some(events) = wait_until(notification, deadline) {
        if events.get() & ACTIVITY.get() != 0 {
            return true;
        }
    }
    false
}

/// Waits until a key interrupt or a config request wakes the loop up, idles and deep sleeps meanwhile.
/// A combo progress times out in `combo_timeout` ms: the loop runs then to drop it, the idle delay starts over.
pub fn wait_for_activity(
    notification: &Notification,
    svc: &mut Services,
    joystick: &mut Option<Joystick>,
    combo_timeout: Option<u32>,
) -> anyhow::Result<()> {
    svc.key_matrix.irq_mut().enable_interrupt()?;
    if let Some(joystick) = joystick {
        joystick.irq_mut().enable_interrupt()?;
    }

    if let Some(ms) = combo_timeout {
        activity_before(notification, Instant::now() + Duration::from_millis(ms.into()));
        return Ok(());
    }

    let idle_since = Instant::now();
    // Config requests wake the loop up too, a submitted config is applied on return.
    if activity_before(notification, idle_since + IDLE_DELAY) {
        return Ok(());
    }

    // Idle connection parameters (slave latency) would slow a config transfer down.
    while svc.config_service.transfer_active() {
        if activity_before(notification, Instant::now() + IDLE_DELAY) {
            return Ok(());
        }
    }
    // An update that stopped coming is off the display before the keyboard idles.
    svc.update_firmware_state()?;

    info!("idle");
    svc.combos.reset();
    svc.keyboard.set_idle();

    let settings = &svc.config.settings;
    let mut retry_at = idle_since;
    let mut display_off = false;
    let mut display_off_failures = 0;
    loop {
        // Use the timeout for an accepted host and keep the original start of inactivity.
        let host_connected = svc.keyboard.host_connected();
        let deep_sleep_s = if host_connected {
            settings.sleep_connected_s
        } else {
            settings.sleep_disconnected_s
        };
        let timeout = Duration::from_secs(deep_sleep_s.into());
        let deadline = (idle_since + timeout).max(retry_at);
        if deadline > Instant::now() {
            // A postponed sleep gets a fresh retry budget if the display is still on.
            display_off_failures = 0;
        }
        if let Some(events) = wait_until(notification, deadline) {
            if events.get() & ACTIVITY.get() != 0 {
                break;
            }
            // The host may have disconnected and ended a config session that postponed sleep.
            retry_at = idle_since;
            continue;
        }
        if svc.keyboard.host_connected() != host_connected {
            retry_at = idle_since;
            continue;
        }
        // A wake-up from deep sleep is a reset, it would drop a firmware pending verification.
        if !svc.config_service.session_active() && !firmware::pending_verification() {
            if display_off || display_off_failures >= DISPLAY_OFF_MAX_ATTEMPTS {
                info!("entering deep sleep");
                enter_deep_sleep(&wakeup_pins(&svc.key_matrix, joystick));
            }
            match svc.display.turn_off() {
                Ok(()) => display_off = true,
                Err(e) => {
                    display_off_failures += 1;
                    error!("display off before deep sleep (attempt {}): {:?}", display_off_failures, e);
                }
            }
            // Recheck for events that arrived while the display thread was turning the OLED off.
            continue;
        }
        info!("deep sleep postponed: the config app is connected or the firmware is not confirmed yet");
        // Session pings leave BLE idle. Wait before checking again while sleep is blocked.
        retry_at = Instant::now() + timeout;
    }

    svc.keyboard.set_active();
    info!("active");
    Ok(())
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

pub fn prepare_light_sleep(irq_pins: &[&PinDriver<'static, Input>]) {
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

pub fn prepare_deep_sleep(irq_pins: &[&PinDriver<'static, Input>]) {
    unsafe { 
        for irq_pin in irq_pins {
            esp_idf_sys::gpio_hold_dis(irq_pin.pin() as i32);
        }
        esp_idf_sys::gpio_deep_sleep_hold_dis();
    }
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

fn enter_deep_sleep(irq_pins: &[&PinDriver<'static, Input>]) -> ! {
    // Auto light sleep leaves its timer wake-up source on: deep sleep would wake up (restart) at once.
    if let Err(e) = configure_pm(false) {
        error!("light sleep off before deep sleep: {:?}", e);
    }
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
        esp_idf_sys::esp_deep_sleep_start()
    }
}

/// `ext_irq` going low wakes the main loop up through `notification`.
pub fn setup_interrupt(ext_irq: &mut PinDriver<'static, Input>, notification: Rc<Notification>) -> anyhow::Result<()> {
    ext_irq.set_interrupt_type(InterruptType::LowLevel)?;

    let pin = ext_irq.pin();
    let notifier = notification.notifier();
    unsafe {
        ext_irq.subscribe(move || {
            notifier.notify_and_yield(ACTIVITY);
            esp_idf_sys::gpio_intr_disable(pin as i32);
        })?;
    }
    Ok(())
}
