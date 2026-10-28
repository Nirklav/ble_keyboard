use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{Input, PinDriver};
use log::{info, warn};
use protocol::config::{JoystickButton, Settings};
use protocol::joystick::{Register, State, ADDRESS, AXIS_MAX, VERSION};
use protocol::motion::{Ramp, Velocity};

use crate::i2c_bus::SharedI2c;

const TIMEOUT: TickType = TickType::new_millis(20);

/// Joystick extension, see [`protocol::joystick`].
/// It holds `INT` low while there is a state to read, so it's polled only then.
pub struct Joystick {
    i2c: SharedI2c,
    irq: PinDriver<'static, Input>,
    /// Cursor speed, ramps up while the stick is deflected.
    speed: Ramp,
    /// Held buttons, like [`State::buttons`].
    buttons: u8,
}

impl Joystick {
    /// Returns `None` if the joystick doesn't answer or speaks another protocol version.
    pub fn new(i2c: SharedI2c, irq: PinDriver<'static, Input>) -> Option<Self> {
        let mut version = [0];
        match i2c.write_read(ADDRESS, &[Register::Version as u8], &mut version, TIMEOUT.ticks()) {
            Ok(()) if version[0] == VERSION => {
                info!("joystick connected");
                Some(Self {
                    i2c,
                    irq,
                    speed: Ramp::default(),
                    buttons: 0,
                })
            },
            Ok(()) => {
                warn!("joystick protocol version {} is not supported", version[0]);
                None
            },
            Err(_) => {
                info!("joystick not connected");
                None
            }
        }
    }

    pub fn irq(&self) -> &PinDriver<'static, Input> {
        &self.irq
    }

    pub fn irq_mut(&mut self) -> &mut PinDriver<'static, Input> {
        &mut self.irq
    }

    /// Reads the state if `INT` is asserted: adds the cursor speed to `velocity` as the joystick `settings`
    /// say at `now` ms, passes every button that went down or up to `button_change`,
    /// like [`KeyMatrix::register`](crate::key_matrix::KeyMatrix::register).
    /// Returns `true` while the joystick is in use, so the keyboard must not go idle.
    pub fn poll<F>(&mut self, settings: &Settings, now: u32, velocity: &mut Velocity, mut button_change: F) -> bool
        where F: FnMut(JoystickButton, bool) {
        // `INT` is released: nothing is pressed and the last state is read.
        if self.irq.is_high() {
            self.buttons(0, &mut button_change);
            self.speed.reset();
            return false;
        }

        // Keeps the held buttons on a failure, otherwise the host would see them released.
        if let Err(e) = self.register(settings, now, velocity, &mut button_change) {
            warn!("joystick read failed: {:?}", e);
        }
        true
    }

    fn register(&mut self, settings: &Settings, now: u32, velocity: &mut Velocity, button_change: &mut impl FnMut(JoystickButton, bool)) -> anyhow::Result<()> {
        let mut bytes = [0; State::SIZE];
        self.i2c.write_read(ADDRESS, &[Register::State as u8], &mut bytes, TIMEOUT.ticks())?;
        let state = State::decode(&bytes);

        self.buttons(state.buttons, button_change);

        let (x, y) = target_speed(state.x, state.y, settings.joystick_speed as i32);
        let x = if settings.joystick_invert_x { -x } else { x };
        let y = if settings.joystick_invert_y { -y } else { y };
        let (x, y) = self.speed.follow((x, y), settings.joystick_ramp_ms, now);
        *velocity += Velocity { x, y, wheel: 0 };

        Ok(())
    }

    /// Passes the buttons that differ from the held ones.
    fn buttons(&mut self, buttons: u8, button_change: &mut impl FnMut(JoystickButton, bool)) {
        for button in JoystickButton::ALL {
            let mask = 1 << button.bit();
            let pressed = buttons & mask != 0;
            if pressed != (self.buttons & mask != 0) {
                button_change(button, pressed);
            }
        }
        self.buttons = buttons;
    }
}

/// Converts the stick deflection to the cursor speed in pixels per second, `max_speed` at full deflection.
/// Quadratic on the deflection length, so small deflections are precise. The length
/// is clamped to [`AXIS_MAX`], so the speed is the same in any direction, diagonals included.
fn target_speed(x: i16, y: i16, max_speed: i32) -> (i32, i32) {
    let (x, y) = (x as f32, y as f32);
    let length = (x * x + y * y).sqrt();
    if length == 0.0 {
        return (0, 0);
    }

    let deflection = (length / AXIS_MAX as f32).min(1.0);
    let speed = deflection * deflection * max_speed as f32;
    ((x / length * speed) as i32, (y / length * speed) as i32)
}
