use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{Input, PinDriver};
use log::{info, warn};
use protocol::joystick::{Register, State, ADDRESS, AXIS_MAX, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_STICK, VERSION};

use crate::bt::MouseKey;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::i2c_bus::SharedI2c;

const TIMEOUT: TickType = TickType::new_millis(20);

/// Cursor travel at full deflection, in pixels per poll.
const MAX_SPEED: i32 = 12;

/// Sub-pixel accumulator scale, so slow movement isn't lost to rounding.
const SUB_PIXEL: i32 = 256;

const INVERT_X: bool = false;
const INVERT_Y: bool = false;

/// Joystick extension, see [`protocol::joystick`].
/// It holds `INT` low while there is a state to read, so it's polled only then.
pub struct Joystick {
    i2c: SharedI2c,
    irq: PinDriver<'static, Input>,
    remainder_x: i32,
    remainder_y: i32,
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
                    remainder_x: 0,
                    remainder_y: 0,
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

        let dx = travel(state.x, INVERT_X, &mut self.remainder_x);
        let dy = travel(state.y, INVERT_Y, &mut self.remainder_y);
        keyboard.travel_mouse_cursor(dx, dy);

        Ok(())
    }
}

/// Converts an axis to pixels per poll. Quadratic, so small deflections are precise.
fn travel(axis: i16, invert: bool, remainder: &mut i32) -> i8 {
    if axis == 0 {
        *remainder = 0;
        return 0;
    }

    let axis = if invert { -(axis as i32) } else { axis as i32 };
    let max = AXIS_MAX as i32;
    *remainder += axis * axis.abs() * MAX_SPEED * SUB_PIXEL / (max * max);

    let pixels = *remainder / SUB_PIXEL;
    *remainder -= pixels * SUB_PIXEL;
    pixels.clamp(i8::MIN as i32, i8::MAX as i32) as i8
}
