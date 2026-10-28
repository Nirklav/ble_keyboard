//! Pointer motion over time: speeds are per second, the firmware turns them into the relative
//! values of each mouse report with the time since the previous one, so the cursor and the wheel move
//! as fast whatever the report rate is.

use crate::config::KeyAction;

/// [`KeyAction::MouseTravel`] moves `dx`, `dy` pixels per this many ms.
pub const TRAVEL_UNIT_MS: i32 = 10;

/// Time counted for the first report of a movement, so a press moves at once.
pub const FIRST_STEP_MS: u32 = 10;

/// The longest time counted between two reports: a stalled loop doesn't make the cursor jump.
pub const MAX_STEP_MS: u32 = 50;

/// Time since `last` counted for a step at `now` (wrapping), see [`FIRST_STEP_MS`] and [`MAX_STEP_MS`].
fn elapsed(last: Option<u32>, now: u32) -> u32 {
    last.map_or(FIRST_STEP_MS, |last| now.wrapping_sub(last).min(MAX_STEP_MS))
}

/// Pointer speed: cursor in pixels per second, wheel in steps per second (positive is up).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Velocity {
    pub x: i32,
    pub y: i32,
    pub wheel: i32,
}

impl Velocity {
    /// Speed of a held [`KeyAction::MouseTravel`] or [`KeyAction::MouseScroll`], zero for other actions.
    pub fn of(action: &KeyAction) -> Velocity {
        let per_second = 1000 / TRAVEL_UNIT_MS;
        match *action {
            KeyAction::MouseTravel { dx, dy } => {
                Velocity { x: dx as i32 * per_second, y: dy as i32 * per_second, wheel: 0 }
            }
            KeyAction::MouseScroll { dy } => Velocity { x: 0, y: 0, wheel: dy as i32 },
            _ => Velocity::default(),
        }
    }

    pub fn is_zero(&self) -> bool {
        *self == Velocity::default()
    }
}

impl core::ops::AddAssign for Velocity {
    fn add_assign(&mut self, other: Velocity) {
        self.x = self.x.saturating_add(other.x);
        self.y = self.y.saturating_add(other.y);
        self.wheel = self.wheel.saturating_add(other.wheel);
    }
}

/// Relative values of one mouse report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Step {
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
}

/// Turns a [`Velocity`] into report [`Step`]s, keeping the fractions for the next reports.
#[derive(Debug, Clone, Default)]
pub struct Motion {
    /// Time of the previous step while moving.
    last: Option<u32>,
    /// Fractions not sent yet, in units * ms / 1000.
    remainder: [i32; 3],
    /// Wheel speed of the previous step.
    wheel: i32,
}

impl Motion {
    /// Step of a report at `now` ms (wrapping) with `velocity` since the previous one, see [`FIRST_STEP_MS`]
    /// and [`MAX_STEP_MS`]. A wheel that starts or turns sends one step at once, so a tap of a slow scroll key
    /// scrolls too.
    pub fn step(&mut self, velocity: Velocity, now: u32) -> Step {
        if velocity.is_zero() {
            *self = Motion::default();
            return Step::default();
        }
        let elapsed = elapsed(self.last, now);
        self.last = Some(now);
        if velocity.wheel.signum() != self.wheel.signum() {
            self.remainder[2] = velocity.wheel.signum() * 1000;
        }
        self.wheel = velocity.wheel;

        let mut out = [0i8; 3];
        for (axis, speed) in [velocity.x, velocity.y, velocity.wheel].into_iter().enumerate() {
            let total = self.remainder[axis] + speed * elapsed as i32;
            let units = total / 1000;
            self.remainder[axis] = total - units * 1000;
            out[axis] = units.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        }
        Step { x: out[0], y: out[1], wheel: out[2] }
    }
}

/// Speed that follows a target speed over `ramp_ms`: closes the gap by `elapsed / ramp_ms` per step,
/// and stops at once when the target is zero. Smooths the joystick cursor.
#[derive(Debug, Clone, Default)]
pub struct Ramp {
    speed: [i32; 2],
    last: Option<u32>,
}

impl Ramp {
    /// Speed at `now` ms (wrapping) towards `target` (x, y).
    pub fn follow(&mut self, target: (i32, i32), ramp_ms: u16, now: u32) -> (i32, i32) {
        let elapsed = elapsed(self.last, now);
        self.last = Some(now);
        for (speed, target) in self.speed.iter_mut().zip([target.0, target.1]) {
            *speed = if target == 0 || elapsed >= ramp_ms as u32 {
                target
            } else {
                let step = (target - *speed) * elapsed as i32 / ramp_ms as i32;
                // Without this, a slow target would never be reached because of the rounding.
                if step == 0 { target } else { *speed + step }
            };
        }
        (self.speed[0], self.speed[1])
    }

    /// Starts over: the next target is followed from zero.
    pub fn reset(&mut self) {
        *self = Ramp::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sum of the steps of `reports` reports every `period` ms, and the time they count.
    fn travel(velocity: Velocity, period: u32, reports: u32) -> ((i32, i32, i32), i32) {
        let mut motion = Motion::default();
        let mut sum = (0, 0, 0);
        for report in 0..reports {
            let step = motion.step(velocity, 1000 + report * period);
            sum = (sum.0 + step.x as i32, sum.1 + step.y as i32, sum.2 + step.wheel as i32);
        }
        (sum, (FIRST_STEP_MS + (reports - 1) * period) as i32)
    }

    #[test]
    fn distance_doesnt_depend_on_the_report_rate() {
        let velocity = Velocity { x: 1000, y: -250, wheel: 7 };
        for period in [5, 8, 10, 13, 20] {
            let ((x, y, wheel), ms) = travel(velocity, period, 2000 / period);
            // Whole units of the counted time, the fraction waits for the next report.
            assert_eq!(x, 1000 * ms / 1000, "period {period}");
            assert_eq!(y, -250 * ms / 1000, "period {period}");
            // Plus the step at once of a starting wheel.
            assert_eq!(wheel, 1 + 7 * ms / 1000, "period {period}");
        }
    }

    #[test]
    fn first_step_moves_and_stop_resets() {
        let mut motion = Motion::default();
        let velocity = Velocity::of(&KeyAction::MouseTravel { dx: 3, dy: 0 });
        assert_eq!(velocity, Velocity { x: 300, y: 0, wheel: 0 });
        assert_eq!(motion.step(velocity, 1000), Step { x: 3, y: 0, wheel: 0 });
        // A stalled loop counts at most MAX_STEP_MS.
        assert_eq!(motion.step(velocity, 5000).x, (300 * MAX_STEP_MS / 1000) as i8);
        assert_eq!(motion.step(Velocity::default(), 5010), Step::default());
        assert_eq!(motion.step(velocity, 9000).x, 3);
    }

    #[test]
    fn slow_speed_isnt_lost_to_rounding() {
        // 1 step per second over 8 ms reports, after the step at once.
        let ((_, _, wheel), ms) = travel(Velocity { x: 0, y: 0, wheel: 1 }, 8, 376);
        assert_eq!((wheel, ms), (1 + 3, 3010));
    }

    #[test]
    fn wheel_steps_at_once_when_it_starts_or_turns() {
        let mut motion = Motion::default();
        let up = Velocity::of(&KeyAction::MouseScroll { dy: 2 });
        assert_eq!(motion.step(up, 1000).wheel, 1, "a tap scrolls");
        assert_eq!(motion.step(up, 1008).wheel, 0);
        let down = Velocity { wheel: -2, ..up };
        assert_eq!(motion.step(down, 1016).wheel, -1, "turning scrolls at once too");
        // The cursor moving meanwhile doesn't restart the wheel.
        assert_eq!(motion.step(Velocity { x: 100, ..down }, 1024).wheel, 0);
        assert_eq!(motion.step(Velocity::default(), 1032), Step::default());
        assert_eq!(motion.step(up, 2000).wheel, 1);
    }

    #[test]
    fn ramp_takes_its_time() {
        for period in [5, 10, 20] {
            let mut ramp = Ramp::default();
            let mut now = 0;
            let mut speed = (0, 0);
            while now < 40 {
                now += period;
                speed = ramp.follow((1000, -1000), 40, now);
            }
            assert!(speed.0 > 500 && speed.0 < 1000, "period {period}: {speed:?}");
            while now < 400 {
                now += period;
                speed = ramp.follow((1000, -1000), 40, now);
            }
            assert_eq!(speed, (1000, -1000), "period {period}");
        }
        let mut ramp = Ramp::default();
        assert_eq!(ramp.follow((500, 0), 0, 0), (500, 0));
        assert_eq!(ramp.follow((0, 0), 100, 10), (0, 0));
    }
}
