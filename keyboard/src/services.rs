use log::info;
use protocol::config::Config;
use protocol::keymap::{BtCommand, Keymap};

use crate::config::Pending;

use crate::key_matrix::KeyMatrix;
use crate::display::Display;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;

pub struct Services<const W: usize, const H: usize> {
    pub keyboard: BleMouseKeyboard,
    pub key_matrix: KeyMatrix<W, H>,
    /// What the keys do, made from `config`.
    pub keymap: Keymap,
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
            keymap: Keymap::new(&config),
            display,
            config,
            pending_config: None
        }
    }

    /// Shows the top layer and the active BLE profile once there is no pop-up text.
    pub fn update_status(&self) -> anyhow::Result<()> {
        let layer = &self.keymap.layout(self.keymap.top_layer()).name;
        self.display.set_status(layer, self.keyboard.profile().index)
    }

    /// Runs a BLE profile command of a key or a combo and shows the profile, e.g. `BT 2 pair`.
    pub fn bt_command(&mut self, command: BtCommand) -> anyhow::Result<()> {
        info!("BT command: {:?}", command);
        let keyboard = &mut self.keyboard;
        let (state, cleared) = match command {
            BtCommand::Select(index) => (keyboard.select_profile(index), false),
            BtCommand::Next => (keyboard.step_profile(1), false),
            BtCommand::Prev => (keyboard.step_profile(-1), false),
            BtCommand::Clear => (keyboard.clear_profile(), true),
            BtCommand::ClearAll => {
                keyboard.clear_all_profiles();
                return self.display.show_text("BT clear all");
            },
        };
        // At most 18 characters of the display font.
        let number = state.index + 1;
        let text = match (cleared, state.bound) {
            (true, _) => format!("BT {number} clear"),
            (false, true) => format!("BT {number}"),
            (false, false) => format!("BT {number} pair"),
        };
        self.display.show_text(&text)
    }
}
