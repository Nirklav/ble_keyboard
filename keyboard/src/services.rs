use protocol::config::Config;

use crate::config::Pending;

use crate::key_matrix::KeyMatrix;
use crate::display::Display;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;

pub struct Services<const W: usize, const H: usize> {
    pub keyboard: BleMouseKeyboard,
    pub key_matrix: KeyMatrix<W, H>,
    pub display: Display,
    /// Active config.
    pub config: Config,
    /// Config change to make once all keys are up.
    pub pending_config: Option<Pending>
}

impl<const W: usize, const H: usize> Services<W, H> {
    pub fn new(keyboard: BleMouseKeyboard,
        key_matrix: KeyMatrix<W, H>,
        display: Display,
        config: Config) -> Self {
        Self {
            keyboard,
            key_matrix,
            display,
            config,
            pending_config: None
        }
    }
}
