//! I2C protocol between the keyboard (bus master) and the joystick extension (slave).
//!
//! Register based: the master writes one byte with a [`Register`] address, then
//! reads the register's fixed-size payload (`write_read` transaction).
//! Multi-byte values are little-endian.

/// 7-bit I2C address of the joystick.
pub const ADDRESS: u8 = 0x42;

/// Protocol version, returned by [`Register::Version`].
pub const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    UnknownRegister(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Register {
    /// 1 byte: [`VERSION`].
    Version = 0x00,
    /// [`State::SIZE`] bytes: encoded [`State`].
    State = 0x01,
}

impl Register {
    /// Number of bytes the master reads from this register.
    pub const fn size(self) -> usize {
        match self {
            Register::Version => 1,
            Register::State => State::SIZE,
        }
    }
}

impl TryFrom<u8> for Register {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Error> {
        match value {
            0x00 => Ok(Register::Version),
            0x01 => Ok(Register::State),
            other => Err(Error::UnknownRegister(other)),
        }
    }
}

/// Snapshot of the joystick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// Horizontal axis, 0 is center.
    pub x: i16,
    /// Vertical axis, 0 is center.
    pub y: i16,
    /// Bit mask of pressed buttons, bit N is button N.
    pub buttons: u8,
}

impl State {
    pub const SIZE: usize = 5;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let x = self.x.to_le_bytes();
        let y = self.y.to_le_bytes();
        [x[0], x[1], y[0], y[1], self.buttons]
    }

    pub fn decode(bytes: &[u8; Self::SIZE]) -> Self {
        Self {
            x: i16::from_le_bytes([bytes[0], bytes[1]]),
            y: i16::from_le_bytes([bytes[2], bytes[3]]),
            buttons: bytes[4],
        }
    }

    pub fn is_pressed(&self, button: u8) -> bool {
        button < 8 && self.buttons & (1 << button) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip() {
        let state = State { x: -1234, y: i16::MAX, buttons: 0b101 };
        assert_eq!(State::decode(&state.encode()), state);
        assert!(state.is_pressed(0));
        assert!(!state.is_pressed(1));
        assert!(state.is_pressed(2));
    }

    #[test]
    fn register_roundtrip() {
        for register in [Register::Version, Register::State] {
            assert_eq!(Register::try_from(register as u8), Ok(register));
        }
        assert_eq!(Register::try_from(0xFF), Err(Error::UnknownRegister(0xFF)));
    }
}
