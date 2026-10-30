use log::info;
use protocol::config::Config;
use protocol::keymap::{BtCommand, Keymap};

use crate::config::Pending;
use crate::firmware;

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
        mut key_matrix: KeyMatrix<W, H>,
        display: Display,
        config: Config) -> Self {
        key_matrix.set_debounce_ms(config.settings.debounce_ms);
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

    /// While the chunks of a firmware update come: shows its progress and asks for the shortest connection
    /// interval. An update the app stopped sending without ending it counts only until the transfer times out.
    pub fn update_firmware_state(&mut self) -> anyhow::Result<()> {
        let progress = firmware::update_progress().filter(|_| self.keyboard.config_service().transfer_active());
        // The image check after the last chunk takes no requests.
        self.keyboard.set_transfer(progress.is_some_and(|progress| progress < 100));
        self.display.set_update(progress.is_some())
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
        // At most 17 characters fit the pop-up plate.
        let number = state.index + 1;
        let text = match (cleared, state.bound) {
            (true, _) => format!("BT {number} clear"),
            (false, true) => format!("BT {number}"),
            (false, false) => format!("BT {number} pair"),
        };
        self.display.show_text(&text)
    }
}
