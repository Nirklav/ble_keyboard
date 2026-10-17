use std::sync::{Arc, Mutex};
use std::time::Duration;

use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::timer::{EspTaskTimerService, EspTimer};
use log::error;
use ssd1306::{I2CDisplayInterface, size::DisplaySize128x32};
use ssd1306::prelude::{DisplayRotation, I2CInterface};
use ssd1306::mode::{BufferedGraphicsMode, DisplayConfig};

const SCREEN_TEXT_TIMEOUT: Duration = Duration::from_secs(5);

pub type Ssd1306 = ssd1306::Ssd1306<I2CInterface<I2cDriver<'static>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

pub struct Display {
    inner: Inner,
    text_style: MonoTextStyle<'static, BinaryColor>,
    clear_timer: EspTimer<'static>
}

impl Display {
    pub fn new(i2c_driver: I2cDriver<'static>) -> anyhow::Result<Display> {
        let i2c_display = I2CDisplayInterface::new(i2c_driver);
        let timer_service = EspTaskTimerService::new()?;

        let mut display = ssd1306::Ssd1306::new(i2c_display, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        map_err!(display.init());

        let inner = Inner {
            display: Arc::new(Mutex::new(display))
        };

        let inner_clone = inner.clone();
        let clear_timer = timer_service.timer(move || {
            if let Err(e) = inner_clone.clear() {
                error!("error on clear: {:?}", e);
            }
        })?;

        Ok(Self {
            inner,
            text_style: MonoTextStyleBuilder::new()
                .font(&FONT_10X20)
                .text_color(BinaryColor::On)
                .build(),
            clear_timer
        })
    }

    pub fn show_text(&self, text: &str) -> anyhow::Result<()> {
        let mut display = match self.inner.display.lock() {
            Ok(g) => g,
            Err(_) => return Err(anyhow::Error::msg("poison"))
        };

        let text_style = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Middle)
            .build();

        map_err!(display.set_display_on(true));
        map_err!(display.clear(BinaryColor::Off));
        map_err!(Text::with_text_style(text, Point::new(64, 16), self.text_style, text_style)
            .draw(&mut *display));

        map_err!(display.flush());

        self.clear_after(SCREEN_TEXT_TIMEOUT)?;
        Ok(())
    }

    pub fn clear_after(&self, duration: Duration) -> anyhow::Result<()> {
        if self.clear_timer.is_scheduled()? {
            self.clear_timer.cancel()?;
        }

        self.clear_timer.after(duration)?;
        Ok(())
    }

    #[allow(unused)]
    pub fn clear(&self) -> anyhow::Result<()> {
        self.inner.clear()
    }
}

#[derive(Clone)]
struct Inner {
    display: Arc<Mutex<Ssd1306>>,
}

impl Inner {
    pub fn clear(&self) -> anyhow::Result<()> {
        let mut display = match self.display.lock() {
            Ok(g) => g,
            Err(_) => return Err(anyhow::Error::msg("poison"))
        };
        map_err!(display.clear(BinaryColor::Off));
        map_err!(display.flush());
        map_err!(display.set_display_on(false));
        Ok(())
    }
}