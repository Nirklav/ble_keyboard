use log::{error, info};
use protocol::combos::{ComboEvent, Combos};
use protocol::config::{ComboAction, Config};
use protocol::keymap::{BtCommand, Keymap};

use crate::config::Pending;
use crate::config_service::ConfigService;
use crate::firmware;

use crate::hardware::Matrix;
use crate::display::Display;
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;

pub struct Services {
    pub keyboard: BleMouseKeyboard,
    /// The config protocol over the HID config reports of `keyboard`.
    pub config_service: ConfigService,
    pub key_matrix: Matrix,
    /// What the keys do, made from `config`.
    pub keymap: Keymap,
    /// Special combinations of `config`.
    pub combos: Combos,
    pub display: Display,
    /// Active config.
    pub config: Config,
    /// Config change to make once all keys are up.
    pub pending_config: Option<Pending>
}

impl Services {
    pub fn new(keyboard: BleMouseKeyboard,
        config_service: ConfigService,
        mut key_matrix: Matrix,
        display: Display,
        config: Config) -> Self {
        key_matrix.set_debounce_ms(config.settings.debounce_ms);
        display.configure(&config);
        Self {
            keyboard,
            config_service,
            key_matrix,
            keymap: Keymap::new(&config),
            combos: Combos::new(&config.combos),
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
        let progress = firmware::update_progress().filter(|_| self.config_service.transfer_active());
        // The image check after the last chunk takes no requests.
        self.keyboard.set_transfer(progress.is_some_and(|progress| progress < 100));
        self.display.set_update(progress.is_some())
    }

    /// Runs `command` and shows what it did.
    pub fn run(&mut self, command: Command) -> anyhow::Result<()> {
        match command {
            Command::SwitchLayout => {
                let new_layout = self.keymap.switch_layout();
                info!("Layout switched to: {}", new_layout);
                self.display.show_text(&self.keymap.layout(new_layout).name)?;
            },
            Command::SetLayout(idx) => {
                if self.keymap.set_base(idx) {
                    info!("Layout set to: {}", idx);
                    self.display.show_text(&self.keymap.layout(idx).name)?;
                } else {
                    error!("SetLayout({}) is out of range, ignored.", idx);
                }
            },
            Command::Bt(command) => self.bt_command(command)?,
            Command::ResetConfig => {
                info!("Config reset requested");
                self.pending_config = Some(Pending::Reset);
                self.display.show_text("Reset")?;
            }
        }
        Ok(())
    }

    /// After the key changes of a scan: shows the presses left of the destructive combos, runs the ones that fired.
    pub fn poll_combos(&mut self) -> anyhow::Result<()> {
        for event in self.combos.poll() {
            match event {
                ComboEvent::Progress { action, left } => self.show_progress(action, left)?,
                ComboEvent::Fired(action) => {
                    if let Some(command) = Command::of(action) {
                        self.run(command)?;
                    }
                },
            }
        }
        Ok(())
    }

    /// Destructive combos count the presses down; at most 18 characters with the number.
    fn show_progress(&self, action: ComboAction, left: u8) -> anyhow::Result<()> {
        let label = match action {
            ComboAction::Unpair => "Unpair",
            ComboAction::ResetConfig => "Reset",
            ComboAction::BtClear => "Clear",
            ComboAction::BtClearAll => "Clear all",
            ComboAction::SwitchLayout
            | ComboAction::SetLayout(_)
            | ComboAction::BtSelect(_)
            | ComboAction::BtNext
            | ComboAction::BtPrev
            | ComboAction::Action(_) => return Ok(())
        };
        self.display.show_text(&format!("{}: {}", label, left))
    }

    /// Runs a BLE profile command of a key or a combo and shows the profile, e.g. `BT 2 pair`.
    fn bt_command(&mut self, command: BtCommand) -> anyhow::Result<()> {
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

/// What a key or a combo asks the firmware to do, besides the reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// [`Keymap::switch_layout`].
    SwitchLayout,
    /// [`Keymap::set_base`].
    SetLayout(u8),
    Bt(BtCommand),
    /// The factory config, once all keys are up (see [`Services::pending_config`]).
    ResetConfig,
}

impl Command {
    /// What a fired special combination does; chords are the keymap's, they do nothing here.
    pub fn of(action: ComboAction) -> Option<Command> {
        Some(match action {
            ComboAction::SwitchLayout => Command::SwitchLayout,
            ComboAction::SetLayout(index) => Command::SetLayout(index),
            // Unpair is what the configs before the BLE profiles have.
            ComboAction::Unpair | ComboAction::BtClear => Command::Bt(BtCommand::Clear),
            ComboAction::ResetConfig => Command::ResetConfig,
            ComboAction::BtSelect(index) => Command::Bt(BtCommand::Select(index)),
            ComboAction::BtNext => Command::Bt(BtCommand::Next),
            ComboAction::BtPrev => Command::Bt(BtCommand::Prev),
            ComboAction::BtClearAll => Command::Bt(BtCommand::ClearAll),
            ComboAction::Action(_) => return None,
        })
    }
}
