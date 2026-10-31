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

use protocol::config::{Config, Settings};

use crate::firmware;
use crate::i2c_bus::SharedI2c;

/// How long a pop-up is shown, the slides in and out included.
const POPUP_TIMEOUT: Duration = Duration::from_secs(5);
/// A pop-up plate slides down from above the screen pushing the status out, and back up.
const POPUP_SLIDE: Duration = Duration::from_millis(160);
/// Between the frames of a slide; a frame itself takes ~13 ms at 400 kHz.
const POPUP_FRAME: Duration = Duration::from_millis(16);
/// The status texts move by a pixel this often, see [`Drift`].
const DRIFT_STEP: Duration = Duration::from_secs(30);
/// At least between the layout name and the profile, a character.
const STATUS_GAP: i32 = 7;

/// Pop-ups (at most 17 characters) and the status: a layout name of at most 12 characters
/// on the left, the profile on the right.
const FONT: &MonoFont = &FONT_7X14;

const WIDTH: i32 = 128;
const HEIGHT: i32 = 32;
const PLATE_HEIGHT: u32 = 24;
/// Between the text and the plate's sides; a long text takes the whole width.
const PLATE_PADDING: u32 = 8;
const PLATE_RADIUS: u32 = 4;
/// How often the progress of a firmware update is checked. The display thread reads it itself: the main loop
/// rarely runs meanwhile, the update thread (of the display's priority) has chunks to handle all the time.
const UPDATE_POLL: Duration = Duration::from_millis(200);
/// Progress bar of a firmware update, under its text.
const BAR_TOP: i32 = 20;
const BAR_HEIGHT: u32 = 10;

/// Of the display thread: drawing, a flush and logging an error took the 3.5 KB esp_timer task before,
/// the frame buffer (512 bytes) is on this stack now.
const STACK_SIZE: usize = 6 * 1024;

pub type Ssd1306 = ssd1306::Ssd1306<I2CInterface<SharedI2c>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

/// Shows pop-up text on a plate for a while, then the status until [`Settings::display_off_s`] passes without key
/// presses or changes, an OLED burns in; all at the lowest brightness.
/// Drawn by its own thread: a frame holds the I2C bus for ~13 ms, the callers only send it what to show.
pub struct Display {
    sender: Sender<Msg>,
    /// The main loop's copy of the status: an unchanged one isn't sent.
    status: Mutex<Status>,
    /// The main loop's copy of whether a firmware update is shown, likewise.
    update: Mutex<bool>,
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
            updating: false,
            update_drawn: None,
            update_changed: false,
            off_at: None,
            timeout: Duration::from_secs(Settings::default().display_off_s.into()),
            longest_layer: 0,
            on: false,
            layer_drift: Drift::random(),
            profile_drift: Drift::random(),
            drift_at: Instant::now()
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
            update: Mutex::new(false),
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

    /// A firmware update is received: its progress ([`firmware::update_progress`]) is shown instead of anything
    /// else until it's turned off.
    pub fn set_update(&self, updating: bool) -> anyhow::Result<()> {
        let mut update = lock(&self.update)?;
        if *update == updating {
            return Ok(());
        }
        *update = updating;
        self.send(Msg::Update(updating))
    }

    /// Of a new `config`: a shown status goes off [`Settings::display_off_s`] after now,
    /// the status texts drift in parts that fit its longest layout name.
    pub fn configure(&self, config: &Config) {
        let timeout = Duration::from_secs(config.settings.display_off_s.into());
        let longest_layer = config.layouts.iter().map(|layout| text_width(&layout.name)).max().unwrap_or(0);
        // Never fails, see `send`.
        let _ = self.send(Msg::Configure { timeout, longest_layer });
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
    Update(bool),
    Configure { timeout: Duration, longest_layer: u32 },
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

/// Where a status text is in its part of the screen, a pixel further each [`DRIFT_STEP`], so the same pixels aren't
/// lit for hours: back and forth across it, a row lower or higher at each side, every position of the part in turn.
/// Kept as a phase: a part that changes with the layout name keeps the text inside it.
struct Drift {
    phase: u32
}

impl Drift {
    /// Anywhere: deep sleep restarts the chip, always starting at the same place would wear it out the most.
    /// Random only with the radio on, see `esp_random`.
    fn random() -> Self {
        Self { phase: unsafe { esp_idf_sys::esp_random() } }
    }

    fn step(&mut self) {
        self.phase = self.phase.wrapping_add(1);
    }

    /// From `0` to `width` and `height` inclusive.
    fn offset(&self, width: i32, height: i32) -> Point {
        // A sweep across a row, then a step to the next one at its side.
        let positions = width.max(0) as u32 + 1;
        let sweep = self.phase / positions;
        let along = (self.phase % positions) as i32;
        let x = if sweep % 2 == 0 { along } else { width - along };
        Point::new(x, triangle(sweep, height))
    }
}

fn triangle(phase: u32, range: i32) -> i32 {
    if range <= 0 {
        return 0;
    }
    let range = range as u32;
    let phase = phase % (2 * range);
    phase.min(2 * range - phase) as i32
}

/// Owned by the display thread, nothing else touches the driver.
struct Screen {
    display: Ssd1306,
    status: Status,
    status_changed: bool,
    woken: bool,
    /// Shown instead of the status until it's over, the status is drawn then.
    popup: Option<Popup>,
    /// A firmware update is shown instead of the pop-ups and the status, which never goes off meanwhile.
    updating: bool,
    /// Its progress on the screen.
    update_drawn: Option<u8>,
    update_changed: bool,
    /// When the status goes off; not counted under a pop-up, it starts over after one.
    off_at: Option<Instant>,
    /// Of the status, see [`Display::configure`].
    timeout: Duration,
    /// Width of the longest layout name, see [`Display::configure`].
    longest_layer: u32,
    on: bool,
    layer_drift: Drift,
    profile_drift: Drift,
    /// The next step of the drifts while the status is on.
    drift_at: Instant
}

impl Screen {
    fn run(mut self, receiver: Receiver<Msg>) {
        loop {
            let next_at = match &self.popup {
                _ if self.updating => Some(Instant::now() + UPDATE_POLL),
                Some(popup) => Some(popup.frame_at),
                None => self.off_at.map(|at| at.min(self.drift_at)),
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
            Msg::Update(updating) => {
                self.updating = updating;
                self.update_drawn = None;
                self.update_changed = true;
            },
            Msg::Configure { timeout, longest_layer } => {
                self.longest_layer = longest_layer;
                self.timeout = timeout;
                if let Some(at) = &mut self.off_at {
                    *at = Instant::now() + timeout;
                }
            },
            Msg::Wake => self.woken = true,
            Msg::Off(ack) => {
                // Nothing that came before comes back on.
                self.popup = None;
                self.updating = false;
                self.update_drawn = None;
                self.update_changed = false;
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
        let update_changed = mem::take(&mut self.update_changed);
        if self.updating {
            // Pop-ups that came meanwhile are over, the status is drawn after the update (if it fails).
            self.popup = None;
            self.off_at = None;
            // A failed update stays at its last progress until the main loop turns it off.
            let progress = firmware::update_progress().or(self.update_drawn).unwrap_or(0);
            if self.update_drawn == Some(progress) {
                return Ok(());
            }
            // Even if drawing fails: it's tried again on the next change only.
            self.update_drawn = Some(progress);
            return self.draw_update(progress);
        }
        let mut ended = update_changed;
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
            self.off_at = Some(now + self.timeout);
            return self.draw_status();
        }
        if woken {
            self.off_at = Some(now + self.timeout);
        } else if self.off_at.is_some_and(|at| now >= at) {
            self.off_at = None;
            return self.turn_off();
        }
        if self.off_at.is_some() && now >= self.drift_at {
            self.drift_at = now + DRIFT_STEP;
            self.layer_drift.step();
            self.profile_drift.step();
            return self.draw_status();
        }
        Ok(())
    }

    fn draw_status(&mut self) -> anyhow::Result<()> {
        map_err!(self.display.clear(BinaryColor::Off));
        self.draw_status_text(0)?;
        map_err!(self.display.flush());
        self.turn_on()
    }

    /// `shift` moves it down. The layout name drifts in the left part of the screen, the profile in the right one:
    /// the width they leave free is split between them. The parts are of the longest layout name: a layer that comes
    /// and goes (Fn held) moves nothing.
    fn draw_status_text(&mut self, shift: i32) -> anyhow::Result<()> {
        if !self.on {
            // The first frame on: the status comes back elsewhere. Not before, the radio is on by now.
            self.layer_drift = Drift::random();
            self.profile_drift = Drift::random();
        }
        let profile = format!("BT{}", self.status.profile + 1);
        let layer_width = text_width(&self.status.layer).max(self.longest_layer) as i32;
        let profile_width = text_width(&profile) as i32;
        let free = (WIDTH - layer_width - profile_width - STATUS_GAP).max(0);
        let rows = HEIGHT - FONT.character_size.height as i32;
        let layer_at = self.layer_drift.offset(free / 2, rows);
        let profile_at = self.profile_drift.offset(free - free / 2, rows);
        let top_left = TextStyleBuilder::new().baseline(Baseline::Top).build();
        let display = &mut self.display;
        map_err!(Text::with_text_style(&self.status.layer, layer_at + Point::new(0, shift), style(), top_left)
            .draw(display));
        let profile_at = Point::new(WIDTH - profile_width - profile_at.x, profile_at.y + shift);
        map_err!(Text::with_text_style(&profile, profile_at, style(), top_left)
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

        let width = (text_width(text) + 2 * PLATE_PADDING).min(WIDTH as u32);
        let plate = Rectangle::new(Point::new((WIDTH - width as i32) / 2, top), Size::new(width, PLATE_HEIGHT));
        let plate_style = PrimitiveStyleBuilder::new()
            .fill_color(BinaryColor::Off)
            .stroke_color(BinaryColor::On)
            .stroke_width(1)
            .stroke_alignment(StrokeAlignment::Inside)
            .build();

        map_err!(self.display.clear(BinaryColor::Off));
        // All the way down it's off the screen wherever it has drifted to, always under the plate's bottom.
        self.draw_status_text((top - hidden) * HEIGHT / (down - hidden))?;
        let display = &mut self.display;
        map_err!(RoundedRectangle::with_equal_corners(plate, Size::new_equal(PLATE_RADIUS))
            .into_styled(plate_style)
            .draw(display));
        map_err!(Text::with_text_style(text, plate.center(), style(), aligned(Alignment::Center))
            .draw(display));
        map_err!(display.flush());
        // Off, it was cleared before: on only with the first frame drawn.
        self.turn_on()
    }

    /// `Updating NN%` and a bar `progress` percent full.
    fn draw_update(&mut self, progress: u8) -> anyhow::Result<()> {
        let display = &mut self.display;
        map_err!(display.clear(BinaryColor::Off));
        let text = format!("Updating {progress}%");
        map_err!(Text::with_text_style(&text, Point::new(WIDTH / 2, BAR_TOP / 2), style(), aligned(Alignment::Center))
            .draw(display));
        let outline = PrimitiveStyleBuilder::new()
            .stroke_color(BinaryColor::On)
            .stroke_width(1)
            .stroke_alignment(StrokeAlignment::Inside)
            .build();
        map_err!(Rectangle::new(Point::new(0, BAR_TOP), Size::new(WIDTH as u32, BAR_HEIGHT))
            .into_styled(outline)
            .draw(display));
        // Inside the outline, one pixel off it.
        let full = (WIDTH as u32 - 4) * progress.min(100) as u32 / 100;
        let fill = PrimitiveStyleBuilder::new().fill_color(BinaryColor::On).build();
        map_err!(Rectangle::new(Point::new(2, BAR_TOP + 2), Size::new(full, BAR_HEIGHT - 4))
            .into_styled(fill)
            .draw(display));
        map_err!(display.flush());
        self.turn_on()
    }

    /// After a frame is drawn. The drifts step a whole [`DRIFT_STEP`] after the status comes on.
    fn turn_on(&mut self) -> anyhow::Result<()> {
        if !self.on {
            map_err!(self.display.set_display_on(true));
            self.on = true;
            self.drift_at = Instant::now() + DRIFT_STEP;
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

fn text_width(text: &str) -> u32 {
    text.chars().count() as u32 * FONT.character_size.width
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
