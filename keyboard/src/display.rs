use std::mem;
use std::sync::{Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::mono_font::ascii::FONT_7X14;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle, RoundedRectangle, StrokeAlignment};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use log::error;
use ssd1306::{I2CDisplayInterface, size::DisplaySize128x32};
use ssd1306::prelude::{Brightness, DisplayRotation, I2CInterface};
use ssd1306::mode::{BufferedGraphicsMode, DisplayConfig};

use crate::i2c_bus::SharedI2c;

/// How long a pop-up is shown, the slides in and out included.
const POPUP_TIMEOUT: Duration = Duration::from_secs(5);
/// A pop-up plate slides down from above the screen pushing the status out, and back up.
const POPUP_SLIDE: Duration = Duration::from_millis(160);
/// Between the frames of a slide; a frame itself takes ~13 ms at 400 kHz.
const POPUP_FRAME: Duration = Duration::from_millis(16);
/// Without key presses or changes the status goes off, an OLED burns in.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Pop-ups (at most 17 characters) and the status: a layout name of at most 12 characters
/// on the left, the profile on the right.
const FONT: &MonoFont = &FONT_7X14;

const WIDTH: i32 = 128;
const HEIGHT: i32 = 32;
const PLATE_HEIGHT: u32 = 24;
/// Between the text and the plate's sides; a long text takes the whole width.
const PLATE_PADDING: u32 = 8;
const PLATE_RADIUS: u32 = 4;

/// Of the display thread: drawing, a flush and logging an error took the 3.5 KB esp_timer task before,
/// the frame buffer (512 bytes) is on this stack now.
const STACK_SIZE: usize = 6 * 1024;

pub type Ssd1306 = ssd1306::Ssd1306<I2CInterface<SharedI2c>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

/// Shows pop-up text on a plate for a while, then the status until [`STATUS_TIMEOUT`]; all at the lowest brightness.
/// Drawn by its own thread: a frame holds the I2C bus for ~13 ms, the callers only send it what to show.
pub struct Display {
    sender: Sender<Msg>,
    /// The main loop's copy of the status: an unchanged one isn't sent.
    status: Mutex<Status>,
    /// The thread has died, logged once.
    gone: AtomicBool
}

impl Display {
    pub fn new(i2c: SharedI2c) -> anyhow::Result<Display> {
        let i2c_display = I2CDisplayInterface::new(i2c);
        let mut display = ssd1306::Ssd1306::new(i2c_display, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        map_err!(display.init());
        map_err!(display.set_brightness(Brightness::DIMMEST));

        let screen = Screen {
            display,
            status: Status::default(),
            status_changed: false,
            woken: false,
            popup: None,
            off_at: None,
            on: false
        };
        let (sender, receiver) = mpsc::channel();
        // Detached: it ends when the display is dropped.
        std::thread::Builder::new()
            .name("display".into())
            .stack_size(STACK_SIZE)
            .spawn(move || screen.run(receiver))?;

        Ok(Self {
            sender,
            status: Mutex::new(Status::default()),
            gone: AtomicBool::new(false)
        })
    }

    /// A plate already on the screen only changes its text, one on its way in or out comes back from where it is.
    pub fn show_text(&self, text: &str) -> anyhow::Result<()> {
        self.send(Msg::Text(text.to_string()))
    }

    /// Redraws the status if it has changed and no pop-up is shown, otherwise it's shown under and after the pop-up.
    pub fn set_status(&self, layer: &str, profile: u8) -> anyhow::Result<()> {
        let mut status = lock(&self.status)?;
        if status.layer == layer && status.profile == profile {
            return Ok(());
        }
        *status = Status { layer: layer.to_string(), profile };
        self.send(Msg::Status(status.clone()))
    }

    /// A key was pressed or released: the status is shown again, if it has gone off, for another timeout.
    pub fn wake(&self) -> anyhow::Result<()> {
        self.send(Msg::Wake)
    }

    /// Before deep sleep: the display is powered on its own and would keep showing the status.
    /// Returns once it's off, the chip mustn't sleep before.
    pub fn turn_off(&self) -> anyhow::Result<()> {
        let (sender, receiver) = mpsc::channel();
        self.sender.send(Msg::Off(sender)).map_err(|_| anyhow::Error::msg("display thread is gone"))?;
        receiver.recv().map_err(|_| anyhow::Error::msg("display thread is gone"))?
    }

    /// Without the thread the keyboard goes on without the display: the error isn't passed up to the main loop.
    fn send(&self, msg: Msg) -> anyhow::Result<()> {
        if self.sender.send(msg).is_err() && !self.gone.swap(true, Ordering::Relaxed) {
            error!("display thread is gone, nothing is shown");
        }
        Ok(())
    }
}

enum Msg {
    Text(String),
    Status(Status),
    Wake,
    /// Answered once the display is off.
    Off(Sender<anyhow::Result<()>>)
}

#[derive(Default, Clone)]
struct Status {
    layer: String,
    /// Zero-based, shown from 1.
    profile: u8
}

struct Popup {
    text: String,
    shown_at: Instant,
    frame_at: Instant
}

impl Popup {
    /// How much of the plate is down, from 0 (above the screen) to 1; `None` once it's over.
    fn progress(elapsed: Duration) -> Option<f32> {
        let slide = POPUP_SLIDE.as_secs_f32();
        if elapsed < POPUP_SLIDE {
            Some(elapsed.as_secs_f32() / slide)
        } else if elapsed < POPUP_TIMEOUT - POPUP_SLIDE {
            Some(1.0)
        } else if elapsed < POPUP_TIMEOUT {
            Some((POPUP_TIMEOUT - elapsed).as_secs_f32() / slide)
        } else {
            None
        }
    }

    /// Until the next frame: the plate stands still between the slides.
    fn next_frame(elapsed: Duration) -> Duration {
        let hold = POPUP_SLIDE..POPUP_TIMEOUT - POPUP_SLIDE;
        if hold.contains(&elapsed) {
            hold.end - elapsed
        } else {
            POPUP_FRAME
        }
    }
}

/// Owned by the display thread, nothing else touches the driver.
struct Screen {
    display: Ssd1306,
    status: Status,
    status_changed: bool,
    woken: bool,
    /// Shown instead of the status until it's over, the status is drawn then.
    popup: Option<Popup>,
    /// When the status goes off; not counted under a pop-up, it starts over after one.
    off_at: Option<Instant>,
    on: bool
}

impl Screen {
    fn run(mut self, receiver: Receiver<Msg>) {
        loop {
            let next_at = match &self.popup {
                Some(popup) => Some(popup.frame_at),
                None => self.off_at,
            };
            let msg = match next_at {
                Some(at) => receiver.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Whatever came during a flush makes a single frame.
            while let Ok(msg) = receiver.try_recv() {
                self.handle(msg);
            }
            if let Err(e) = self.update() {
                error!("error on display: {:?}", e);
            }
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Text(text) => {
                let now = Instant::now();
                let shown = self.popup.as_ref()
                    .and_then(|popup| Popup::progress(now - popup.shown_at))
                    .unwrap_or(0.0);
                // Back in time by the part of the slide in that is done already.
                let shown_at = now - POPUP_SLIDE.mul_f32(shown);
                self.popup = Some(Popup { text, shown_at, frame_at: now });
            },
            Msg::Status(status) => {
                self.status = status;
                self.status_changed = true;
            },
            Msg::Wake => self.woken = true,
            Msg::Off(ack) => {
                // Nothing that came before comes back on.
                self.popup = None;
                self.status_changed = false;
                self.woken = false;
                self.off_at = None;
                // The caller may have given up.
                let _ = ack.send(self.turn_off());
            },
        }
    }

    /// Draws a due pop-up frame or the status, or turns the status off once it's timed out.
    fn update(&mut self) -> anyhow::Result<()> {
        let now = Instant::now();
        let woken = mem::take(&mut self.woken);
        let mut ended = false;
        if let Some(mut popup) = self.popup.take() {
            let elapsed = now - popup.shown_at;
            if let Some(progress) = Popup::progress(elapsed) {
                if now < popup.frame_at {
                    self.popup = Some(popup);
                    return Ok(());
                }
                // Even if drawing fails: the next frames come, so the pop-up ends.
                popup.frame_at = now + Popup::next_frame(elapsed);
                let drawn = self.draw_popup(&popup.text, progress);
                self.popup = Some(popup);
                return drawn;
            }
            ended = true;
        }

        if ended || self.status_changed || (woken && !self.on) {
            // Even if drawing fails: whatever is shown goes off.
            self.status_changed = false;
            self.off_at = Some(now + STATUS_TIMEOUT);
            return self.draw_status();
        }
        if woken {
            self.off_at = Some(now + STATUS_TIMEOUT);
        } else if self.off_at.is_some_and(|at| now >= at) {
            self.off_at = None;
            return self.turn_off();
        }
        Ok(())
    }

    fn draw_status(&mut self) -> anyhow::Result<()> {
        map_err!(self.display.clear(BinaryColor::Off));
        self.draw_status_text(0)?;
        map_err!(self.display.flush());
        if !self.on {
            map_err!(self.display.set_display_on(true));
            self.on = true;
        }
        Ok(())
    }

    /// `shift` moves it down.
    fn draw_status_text(&mut self, shift: i32) -> anyhow::Result<()> {
        let profile = format!("BT{}", self.status.profile + 1);
        let y = HEIGHT / 2 + shift;
        let display = &mut self.display;
        map_err!(Text::with_text_style(&self.status.layer, Point::new(0, y), style(), aligned(Alignment::Left))
            .draw(display));
        map_err!(Text::with_text_style(&profile, Point::new(WIDTH - 1, y), style(), aligned(Alignment::Right))
            .draw(display));
        Ok(())
    }

    /// The plate `progress` of the way down, it pushes the status down off the screen and pulls it back on the way up;
    /// eased, so it slows down at the bottom on the way in and starts slowly on the way out.
    fn draw_popup(&mut self, text: &str, progress: f32) -> anyhow::Result<()> {
        let eased = 1.0 - (1.0 - progress).powi(3);
        let hidden = -(PLATE_HEIGHT as i32);
        let down = (HEIGHT - PLATE_HEIGHT as i32) / 2;
        let top = hidden + ((down - hidden) as f32 * eased).round() as i32;

        let text_width = text.chars().count() as u32 * FONT.character_size.width;
        let width = (text_width + 2 * PLATE_PADDING).min(WIDTH as u32);
        let plate = Rectangle::new(Point::new((WIDTH - width as i32) / 2, top), Size::new(width, PLATE_HEIGHT));
        let plate_style = PrimitiveStyleBuilder::new()
            .fill_color(BinaryColor::Off)
            .stroke_color(BinaryColor::On)
            .stroke_width(1)
            .stroke_alignment(StrokeAlignment::Inside)
            .build();

        map_err!(self.display.clear(BinaryColor::Off));
        self.draw_status_text(top - hidden)?;
        let display = &mut self.display;
        map_err!(RoundedRectangle::with_equal_corners(plate, Size::new_equal(PLATE_RADIUS))
            .into_styled(plate_style)
            .draw(display));
        map_err!(Text::with_text_style(text, plate.center(), style(), aligned(Alignment::Center))
            .draw(display));
        map_err!(display.flush());
        // Off, it was cleared before: on only with the first frame drawn.
        if !self.on {
            map_err!(display.set_display_on(true));
            self.on = true;
        }
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

fn lock<T>(mutex: &Mutex<T>) -> anyhow::Result<MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| anyhow::Error::msg("poison"))
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
