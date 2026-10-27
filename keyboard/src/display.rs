use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::mono_font::ascii::FONT_7X14;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use esp_idf_svc::timer::{EspTaskTimerService, EspTimer};
use log::error;
use ssd1306::{I2CDisplayInterface, size::DisplaySize128x32};
use ssd1306::prelude::{Brightness, DisplayRotation, I2CInterface};
use ssd1306::mode::{BufferedGraphicsMode, DisplayConfig};

use crate::i2c_bus::SharedI2c;

const SCREEN_TEXT_TIMEOUT: Duration = Duration::from_secs(5);
/// Without key presses or changes the status goes off, an OLED burns in.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Pop-ups (at most 18 characters) and the status: a layout name of at most 12 characters
/// on the left, the profile on the right.
const FONT: &MonoFont = &FONT_7X14;

pub type Ssd1306 = ssd1306::Ssd1306<I2CInterface<SharedI2c>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

/// Shows pop-up text for a while, then the status at the lowest brightness until [`STATUS_TIMEOUT`].
pub struct Display {
    inner: Inner,
    popup_timer: EspTimer<'static>
}

impl Display {
    pub fn new(i2c: SharedI2c) -> anyhow::Result<Display> {
        let i2c_display = I2CDisplayInterface::new(i2c);
        let timer_service = EspTaskTimerService::new()?;

        let mut display = ssd1306::Ssd1306::new(i2c_display, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        map_err!(display.init());

        let state = Arc::new(Mutex::new(State {
            display,
            status: Status::default(),
            popup_until: None,
            on: false
        }));

        let off_state = state.clone();
        let off_timer = timer_service.timer(move || {
            let result = lock(&off_state).and_then(|mut state| {
                if state.popup_until.is_some() {
                    return Ok(());
                }
                state.turn_off()
            });
            if let Err(e) = result {
                error!("error on status off: {:?}", e);
            }
        })?;

        let inner = Inner {
            state,
            off_timer: Arc::new(Mutex::new(off_timer))
        };

        let inner_clone = inner.clone();
        let popup_timer = timer_service.timer(move || {
            if let Err(e) = inner_clone.end_popup() {
                error!("error on status: {:?}", e);
            }
        })?;

        Ok(Self {
            inner,
            popup_timer
        })
    }

    pub fn show_text(&self, text: &str) -> anyhow::Result<()> {
        {
            let mut state = self.inner.lock()?;
            // Set before the timer is armed: a previous pop-up's timer that fires meanwhile sees it's not over.
            state.popup_until = Some(Instant::now() + SCREEN_TEXT_TIMEOUT);
            state.on = true;

            let display = &mut state.display;
            map_err!(display.set_brightness(Brightness::NORMAL));
            map_err!(display.clear(BinaryColor::Off));
            map_err!(Text::with_text_style(text, Point::new(64, 16), style(), aligned(Alignment::Center))
                .draw(display));
            map_err!(display.flush());
            map_err!(display.set_display_on(true));
        }

        self.popup_timer.after(SCREEN_TEXT_TIMEOUT)?;
        Ok(())
    }

    /// Redraws the status if it has changed and no pop-up is shown, otherwise it's shown after the pop-up.
    pub fn set_status(&self, layer: &str, profile: u8) -> anyhow::Result<()> {
        {
            let mut state = self.inner.lock()?;
            if state.status.layer == layer && state.status.profile == profile {
                return Ok(());
            }

            state.status = Status { layer: layer.to_string(), profile };
            if state.popup_until.is_some() {
                return Ok(());
            }
            state.draw_status()?;
        }
        self.inner.status_for_timeout()
    }

    /// A key was pressed or released: the status is shown again, if it has gone off, for another timeout.
    pub fn wake(&self) -> anyhow::Result<()> {
        {
            let mut state = self.inner.lock()?;
            if state.popup_until.is_some() {
                return Ok(());
            }
            if !state.on {
                state.draw_status()?;
            }
        }
        self.inner.status_for_timeout()
    }

    /// Before deep sleep: the display is powered on its own and would keep showing the status.
    pub fn turn_off(&self) -> anyhow::Result<()> {
        self.popup_timer.cancel()?;
        self.inner.off_timer()?.cancel()?;
        self.inner.lock()?.turn_off()
    }
}

#[derive(Default)]
struct Status {
    layer: String,
    /// Zero-based, shown from 1.
    profile: u8
}

struct State {
    display: Ssd1306,
    status: Status,
    /// Pop-up text is shown until then, the status waits for the timer.
    popup_until: Option<Instant>,
    on: bool
}

impl State {
    fn draw_status(&mut self) -> anyhow::Result<()> {
        let profile = format!("BT{}", self.status.profile + 1);
        let display = &mut self.display;

        map_err!(display.set_brightness(Brightness::DIMMEST));
        map_err!(display.clear(BinaryColor::Off));
        map_err!(Text::with_text_style(&self.status.layer, Point::new(0, 16), style(), aligned(Alignment::Left))
            .draw(display));
        map_err!(Text::with_text_style(&profile, Point::new(127, 16), style(), aligned(Alignment::Right))
            .draw(display));
        map_err!(display.flush());
        map_err!(display.set_display_on(true));
        self.on = true;
        Ok(())
    }

    fn turn_off(&mut self) -> anyhow::Result<()> {
        let display = &mut self.display;
        map_err!(display.clear(BinaryColor::Off));
        map_err!(display.flush());
        map_err!(display.set_display_on(false));
        self.on = false;
        Ok(())
    }
}

/// Shared with the pop-up timer, which starts the status timeout.
#[derive(Clone)]
struct Inner {
    state: Arc<Mutex<State>>,
    off_timer: Arc<Mutex<EspTimer<'static>>>,
}

impl Inner {
    fn lock(&self) -> anyhow::Result<MutexGuard<'_, State>> {
        lock(&self.state)
    }

    fn off_timer(&self) -> anyhow::Result<MutexGuard<'_, EspTimer<'static>>> {
        self.off_timer.lock().map_err(|_| anyhow::Error::msg("poison"))
    }

    fn end_popup(&self) -> anyhow::Result<()> {
        let drawn = {
            let mut state = self.lock()?;
            // The timer of a previous pop-up fired while a new one was being shown.
            if state.popup_until.is_some_and(|until| Instant::now() < until) {
                return Ok(());
            }
            state.popup_until = None;
            state.draw_status()
        };
        // Even if drawing failed: whatever is shown goes off.
        self.status_for_timeout()?;
        drawn
    }

    /// Called with the state unlocked, the off timer locks it.
    fn status_for_timeout(&self) -> anyhow::Result<()> {
        self.off_timer()?.after(STATUS_TIMEOUT)?;
        Ok(())
    }
}

fn lock(state: &Mutex<State>) -> anyhow::Result<MutexGuard<'_, State>> {
    state.lock().map_err(|_| anyhow::Error::msg("poison"))
}

fn style() -> MonoTextStyle<'static, BinaryColor> {
    MonoTextStyle::new(FONT, BinaryColor::On)
}

fn aligned(alignment: Alignment) -> embedded_graphics::text::TextStyle {
    TextStyleBuilder::new()
        .alignment(alignment)
        .baseline(Baseline::Middle)
        .build()
}
