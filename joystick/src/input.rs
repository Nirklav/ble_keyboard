//! Stick axes and buttons sampling.

use protocol::joystick::AXIS_MAX;

/// Full scale of the 12-bit ADC.
const ADC_MAX: i32 = 4095;

/// Dead zone around the center, in [`AXIS_MAX`] units.
const DEAD_ZONE: i32 = 80;

/// Number of equal consecutive samples before a button change is accepted.
const DEBOUNCE_SAMPLES: u8 = 5;

/// Maps raw ADC readings of one axis to `-AXIS_MAX..=AXIS_MAX`.
pub struct Axis {
    center: i32,
}

impl Axis {
    /// `center` is the raw reading of the released stick.
    pub fn new(center: u16) -> Self {
        let center = center as i32;
        // A reading far from the middle means the stick wasn't released at power on.
        let center = if (ADC_MAX / 4..=ADC_MAX * 3 / 4).contains(&center) {
            center
        } else {
            ADC_MAX / 2
        };
        Self { center }
    }

    pub fn value(&self, raw: u16) -> i16 {
        let offset = raw as i32 - self.center;
        // The center is not exactly in the middle, so each half is scaled separately.
        let span = if offset >= 0 { ADC_MAX - self.center } else { self.center };
        let value = (offset * AXIS_MAX as i32 / span).clamp(-(AXIS_MAX as i32), AXIS_MAX as i32);

        let magnitude = value.abs();
        if magnitude <= DEAD_ZONE {
            return 0;
        }
        // Rescale so the output starts from 0 at the dead zone edge.
        let magnitude = (magnitude - DEAD_ZONE) * AXIS_MAX as i32 / (AXIS_MAX as i32 - DEAD_ZONE);
        (magnitude * value.signum()) as i16
    }
}

/// Debounces a bit mask of buttons.
pub struct Buttons {
    stable: u8,
    last: u8,
    count: u8,
}

impl Buttons {
    pub fn new() -> Self {
        Self { stable: 0, last: 0, count: 0 }
    }

    /// Feeds one raw sample, returns the debounced mask.
    pub fn update(&mut self, raw: u8) -> u8 {
        if raw != self.last {
            self.last = raw;
            self.count = 0;
        } else if self.count < DEBOUNCE_SAMPLES {
            self.count += 1;
            if self.count == DEBOUNCE_SAMPLES {
                self.stable = raw;
            }
        }
        self.stable
    }
}
