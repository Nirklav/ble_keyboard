use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{Input, PinDriver};
use log::{info, warn};
use protocol::joystick::{Register, State, ADDRESS, AXIS_MAX, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_STICK, VERSION};

use crate::bt::MouseKey;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::i2c_bus::SharedI2c;

const TIMEOUT: TickType = TickType::new_millis(20);

/// Cursor travel at full deflection, in pixels per poll.
const MAX_SPEED: i32 = 18;

/// Sub-pixel accumulator scale, so slow movement isn't lost to rounding.
const SUB_PIXEL: i32 = 256;

const INVERT_X: bool = false;
const INVERT_Y: bool = false;

/// Share of the gap to the target speed closed per poll while the stick is deflected.
const ACCELERATION: i32 = 4;

/// Joystick extension, see [`protocol::joystick`].
/// It holds `INT` low while there is a state to read, so it's polled only then.
pub struct Joystick {
    i2c: SharedI2c,
    irq: PinDriver<'static, Input>,
    x: Inertia,
    y: Inertia,
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
                    x: Inertia::default(),
                    y: Inertia::default(),
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

    /// Applies the state to the mouse report if `INT` is asserted.
    /// Returns `true` while the joystick is in use, so the keyboard must not go idle.
    pub fn poll(&mut self, keyboard: &mut BleMouseKeyboard) -> bool {
        // `INT` is asserted: the state changed or the joystick is in use.
        if self.irq.is_high() {
            return false;
        }

        if let Err(e) = self.register(keyboard) {
            warn!("joystick read failed: {:?}", e);
        }
        true
    }

    fn register(&mut self, keyboard: &mut BleMouseKeyboard) -> anyhow::Result<()> {
        let mut bytes = [0; State::SIZE];
        self.i2c.write_read(ADDRESS, &[Register::State as u8], &mut bytes, TIMEOUT.ticks())?;
        let state = State::decode(&bytes);

        if state.is_pressed(BUTTON_LEFT) {
            keyboard.press_mouse(MouseKey::Left);
        }
        if state.is_pressed(BUTTON_RIGHT) {
            keyboard.press_mouse(MouseKey::Right);
        }
        if state.is_pressed(BUTTON_STICK) {
            keyboard.press_mouse(MouseKey::Middle);
        }

        let (x, y) = target_speed(state.x, state.y);
        let dx = self.x.travel(if INVERT_X { -x } else { x });
        let dy = self.y.travel(if INVERT_Y { -y } else { y });
        keyboard.travel_mouse_cursor(dx, dy);

        Ok(())
    }
}

/// Converts the stick deflection to the cursor speed, in sub-pixels per poll.
/// Quadratic on the deflection length, so small deflections are precise. The length
/// is clamped to [`AXIS_MAX`], so the speed is the same in any direction, diagonals included.
fn target_speed(x: i16, y: i16) -> (i32, i32) {
    let (x, y) = (x as f32, y as f32);
    let length = (x * x + y * y).sqrt();
    if length == 0.0 {
        return (0, 0);
    }

    let deflection = (length / AXIS_MAX as f32).min(1.0);
    let speed = deflection * deflection * (MAX_SPEED * SUB_PIXEL) as f32;
    ((x / length * speed) as i32, (y / length * speed) as i32)
}

/// Cursor speed along one axis, in sub-pixels per poll.
/// It ramps up while the stick is deflected and drops to zero on release.
#[derive(Default)]
struct Inertia {
    speed: i32,
    remainder: i32,
}

impl Inertia {
    /// Moves towards the `target` speed, returns the pixels to travel this poll.
    fn travel(&mut self, target: i32) -> i8 {
        if target == 0 {
            self.speed = 0;
            self.remainder = 0;
            return 0;
        }

        let step = (target - self.speed) / ACCELERATION;
        // Without this, a slow target would never be reached because of the rounding.
        self.speed = if step == 0 { target } else { self.speed + step };

        self.remainder += self.speed;
        let pixels = self.remainder / SUB_PIXEL;
        self.remainder -= pixels * SUB_PIXEL;
        pixels.clamp(i8::MIN as i32, i8::MAX as i32) as i8
    }
}
