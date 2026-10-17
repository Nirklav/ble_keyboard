use crate::layout::Layouts;
use crate::key_matrix::KeyMatrix;
use crate::display::Display;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;

pub struct Services<const W: usize, const H: usize, const C: usize> {
    pub keyboard: BleMouseKeyboard,
    pub key_matrix: KeyMatrix<W, H>,
    pub layouts: &'static Layouts<W, H, C>,
    pub display: Display
}

impl<const W: usize, const H: usize, const C: usize> Services<W, H, C> {
    pub fn new(keyboard: BleMouseKeyboard,
        key_matrix: KeyMatrix<W, H>,
        layouts: &'static Layouts<W, H, C>,
        display: Display) -> Self {
        Self {
            keyboard,
            key_matrix,
            layouts,
            display
        }
    }
}