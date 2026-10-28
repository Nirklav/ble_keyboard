//! I2C protocol between the keyboard (bus master) and the joystick extension (slave).
//!
//! Register based: the master writes one byte with a [`Register`] address, then
//! reads the register's fixed-size payload (`write_read` transaction).
//! Multi-byte values are little-endian.
//!
//! The joystick signals the master over the `INT` line: open-drain, active low,
//! level triggered. `INT` is held low while the joystick has something to report:
//! the [`State`] changed since the master last read it, button changes are left to read
//! (see [`State::buttons`]), or the state is active (stick deflected or a button pressed,
//! see [`State::is_active`]). It is released once the master has read [`Register::State`],
//! every button change included, and the state is idle. So the master
//! keeps polling [`Register::State`] while `INT` is low and stops when it goes high.

/// 7-bit I2C address of the joystick.
pub const ADDRESS: u8 = 0x42;

/// Protocol version, returned by [`Register::Version`].
pub const VERSION: u8 = 1;

/// Axis values are in `-AXIS_MAX..=AXIS_MAX`, the dead zone around the center is already applied.
pub const AXIS_MAX: i16 = 1000;

/// [`State::buttons`] bit of the stick press.
pub const BUTTON_STICK: u8 = 0;
/// [`State::buttons`] bit of the left button.
pub const BUTTON_LEFT: u8 = 1;
/// [`State::buttons`] bit of the right button.
pub const BUTTON_RIGHT: u8 = 2;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// Horizontal axis, 0 is center.
    pub x: i16,
    /// Vertical axis, 0 is center.
    pub y: i16,
    /// Bit mask of pressed buttons, bit N is button N. Every change of the buttons is reported, one change
    /// per read (see [`ButtonQueue`]): a click shorter than the polling period is read as pressed, then as
    /// released, and a double click as two clicks. So the buttons may lag behind the axes, which are always
    /// the latest ones; `INT` stays low while changes are left to read.
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

    /// Stick is deflected or any button is pressed.
    pub fn is_active(&self) -> bool {
        self.x != 0 || self.y != 0 || self.buttons != 0
    }
}

/// Changes of [`State::buttons`] not read by the master yet, oldest first; at most `N` (2 or more).
///
/// The joystick pushes every debounced state, the master reads [`ButtonQueue::next`] and the read
/// change is removed by [`ButtonQueue::ack`] once the read has completed. When it's full, the newest
/// change replaces the last queued one: clicks in between may be lost, but the last state, a final
/// release in particular, never is.
#[derive(Debug, Clone)]
pub struct ButtonQueue<const N: usize> {
    changes: [u8; N],
    len: usize,
    /// Latest pushed buttons.
    latest: u8,
}

impl<const N: usize> ButtonQueue<N> {
    pub const fn new() -> Self {
        assert!(N >= 2);
        Self { changes: [0; N], len: 0, latest: 0 }
    }

    /// Records the latest buttons, queued if they differ from the previous ones.
    pub fn push(&mut self, buttons: u8) {
        if buttons == self.latest {
            return;
        }
        self.latest = buttons;
        if self.len < N {
            self.changes[self.len] = buttons;
            self.len += 1;
            return;
        }
        // Full: the newest change replaces the last one, dropped if it undoes the one before.
        self.changes[N - 1] = buttons;
        if self.changes[N - 2] == buttons {
            self.len -= 1;
        }
    }

    /// Buttons to report next: the oldest queued change, or the latest buttons if there is none.
    /// The flag tells it's a queued change, to pass to [`ButtonQueue::ack`].
    pub fn next(&self) -> (u8, bool) {
        if self.len > 0 {
            (self.changes[0], true)
        } else {
            (self.latest, false)
        }
    }

    /// The master has read what [`ButtonQueue::next`] returned with `queued`; a read queued change is removed.
    /// Pushes in between only add to the end, so it's still the first one.
    pub fn ack(&mut self, queued: bool) {
        if queued && self.len > 0 {
            self.changes.copy_within(1..self.len, 0);
            self.len -= 1;
        }
    }

    /// Changes are left to read.
    pub fn pending(&self) -> bool {
        self.len > 0
    }
}

impl<const N: usize> Default for ButtonQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads like the master until nothing is queued, returns the read buttons.
    fn drain<const N: usize>(queue: &mut ButtonQueue<N>) -> Vec<u8> {
        let mut read = Vec::new();
        while queue.pending() {
            let (buttons, queued) = queue.next();
            read.push(buttons);
            queue.ack(queued);
        }
        read
    }

    #[test]
    fn button_queue_keeps_every_change() {
        let mut queue = ButtonQueue::<8>::new();
        assert_eq!(queue.next(), (0, false));
        // Double click between two reads.
        for buttons in [1, 1, 0, 1, 0, 0] {
            queue.push(buttons);
        }
        assert_eq!(drain(&mut queue), [1, 0, 1, 0]);
        assert_eq!(queue.next(), (0, false));

        // Held: the latest buttons are read once the change has been.
        queue.push(0b10);
        assert_eq!(drain(&mut queue), [0b10]);
        assert_eq!(queue.next(), (0b10, false));
    }

    #[test]
    fn button_queue_unacked_read_is_read_again() {
        let mut queue = ButtonQueue::<4>::new();
        queue.push(1);
        assert_eq!(queue.next(), (1, true));
        // The read failed, a change came meanwhile.
        queue.push(0);
        assert_eq!(queue.next(), (1, true));
        assert_eq!(drain(&mut queue), [1, 0]);

        // Read the latest, a change came before the read completed: it isn't lost.
        let (buttons, queued) = queue.next();
        queue.push(1);
        queue.ack(queued);
        assert_eq!(buttons, 0);
        assert_eq!(drain(&mut queue), [1]);
    }

    #[test]
    fn button_queue_overflow_keeps_the_last_state() {
        let mut queue = ButtonQueue::<4>::new();
        for buttons in [1, 0, 1, 0, 1, 0, 1, 0] {
            queue.push(buttons);
        }
        let read = drain(&mut queue);
        assert_eq!(read.last(), Some(&0));
        assert_eq!(read, [1, 0, 1, 0]);

        for buttons in [1, 0, 1, 0, 1] {
            queue.push(buttons);
        }
        assert_eq!(drain(&mut queue), [1, 0, 1]);
        assert_eq!(queue.next(), (1, false));

        // Another button changing while full replaces the last change.
        for buttons in [0, 1, 0, 1, 0b11] {
            queue.push(buttons);
        }
        assert_eq!(drain(&mut queue), [0, 1, 0, 0b11]);
    }

    #[test]
    fn state_roundtrip() {
        let state = State { x: -1234, y: i16::MAX, buttons: 0b101 };
        assert_eq!(State::decode(&state.encode()), state);
        assert!(state.is_pressed(0));
        assert!(!state.is_pressed(1));
        assert!(state.is_pressed(2));
        assert!(state.is_active());
        assert!(!State::default().is_active());
    }
}
