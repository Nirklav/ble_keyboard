use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::display::Display;
use crate::key_matrix::{KeyChange, KeyMatrix};
use crate::layout::Layouts;

pub mod layout_switch;
pub mod unpair;

use layout_switch::LayoutSwitch;
use unpair::Unpair;

pub enum SC {
    LayoutSwitch(LayoutSwitch),
    Unpair(Unpair),
}

impl SC {
    pub fn new_scan(&mut self) {
        match self {
            SC::LayoutSwitch(s) => s.new_scan(),
            SC::Unpair(s) => s.new_scan(),
        }
    }

    pub fn changed(&mut self, key: &KeyChange) {
        match self {
            SC::LayoutSwitch(s) => s.changed(key),
            SC::Unpair(s) => s.changed(key),
        }
    }

    pub fn apply<const W: usize, const H: usize, const C: usize>(
        &mut self,
        key_matrix: &mut KeyMatrix<W, H>,
        layouts: &'static Layouts<W, H, C>,
        display: &Display,
        bt_keyboard: &mut BleMouseKeyboard,
    ) -> anyhow::Result<()> {
        match self {
            SC::LayoutSwitch(s) => s.apply(key_matrix, layouts, display),
            SC::Unpair(s) => s.apply(bt_keyboard, display),
        }
    }

    pub fn reset(&mut self) {
        match self {
            SC::LayoutSwitch(s) => s.reset(),
            SC::Unpair(s) => s.reset(),
        }
    }
}
