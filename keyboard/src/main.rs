 #[macro_use]
mod map_err;
mod config;
mod config_service;
mod config_store;
mod storage;
mod bt;
mod key_matrix;
mod display;
mod firmware;
mod hardware;
mod i2c_bus;
mod joystick;
mod power;
mod services;

use std::time::Instant;

use esp_idf_svc::hal::delay::Delay;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use protocol::board;
use protocol::keymap::KeyId;
use protocol::motion::{Motion, Velocity};
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;
use crate::config_service::ConfigService;
use crate::hardware::Hardware;
use crate::services::{Command, Services};
// Not used directly: its timer queue is what esp-idf-svc's embassy time driver links against.
use embassy_time as _;

/// Our app description instead of the ESP-IDF one (project `libespidf`): the firmware update checks
/// the project name of an image, see `protocol::image`.
mod app_desc {
    // The macro checks cfgs of other ESP-IDF versions.
    #![allow(unexpected_cfgs)]
    // `true`: build time and date in the right fields (`{}` matches the `()` arm of the macro, which swaps them).
    esp_idf_sys::esp_app_desc!(true);
}

fn main() {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    
    main_body()
        .unwrap()
}

fn main_body() -> anyhow::Result<()> {
    storage::mount()?;

    let Hardware { display, mut key_matrix, notification, mut joystick } = hardware::init()?;

    // Escape also wakes the keyboard up from deep sleep, that's not a request to bypass.
    let bypass = !power::woke_from_deep_sleep() && key_matrix.is_pressed(board::BOOT_KEY.0 as usize, board::BOOT_KEY.1 as usize)?;
    let (config, source) = config::load(board::WIDTH, board::HEIGHT, bypass);
    if let Err(e) = config.validate_for(board::WIDTH, board::HEIGHT) {
        panic!("config is invalid: {}", e);
    }

    let nvs = EspDefaultNvsPartition::take()?;
    let mut keyboard = BleMouseKeyboard::new(nvs)?;
    // Config requests wake the loop up.
    let config_service = ConfigService::register(
        board::WIDTH,
        board::HEIGHT,
        config.to_bytes(),
        notification.notifier(),
        keyboard.config_reports(),
    )?;
    let disconnected = config_service.clone();
    keyboard.start(move || disconnected.on_disconnect())?;
    let delay = Delay::new_default();

    let mut svc = Services::new(keyboard, config_service, key_matrix, display, config);

    // Before the pop-up: its first frames show the status under the plate.
    svc.update_status()?;
    svc.display.show_text(source.message().unwrap_or("Welcome!"))?;

    // While a key is held the loop runs every `DELAY_MS`, that's what lets a tap-hold key become a hold
    // without further events; the keyboard waits for activity only once all keys are up and the keymap is idle.
    const DELAY_MS: u32 = 8;
    let start = Instant::now();
    let mut motion = Motion::default();
    loop {
        // Wraps after 49 days, the keymap compares the times with wrapping.
        let now = start.elapsed().as_millis() as u32;
        let keymap = &mut svc.keymap;
        let mut all_up = !svc.key_matrix.register(|x, y, pressed| keymap.event(KeyId::Matrix { x, y }, pressed, now))?;

        let mut velocity = Velocity::default();
        if let Some(joystick) = &mut joystick {
            let settings = &svc.config.settings;
            all_up &= !joystick.poll(settings, now, &mut velocity, |button, pressed| keymap.event(KeyId::Joystick(button), pressed, now));
        }

        keymap.tick(now);
        keymap.frame();
        for action in keymap.report() {
            // Mouse travel and scroll are speeds, they're turned into the report by the time.
            velocity += Velocity::of(action);
            svc.keyboard.press(action);
        }
        // Speeds are per second: the loop period doesn't change how fast the cursor moves.
        svc.keyboard.travel(motion.step(velocity, now));

        let mut changed = false;
        for change in keymap.take_changes() {
            svc.combos.changed(&change);
            changed = true;
        }
        if changed {
            svc.display.wake()?;
        }

        if keymap.take_layers_changed() {
            let name = &keymap.layout(keymap.top_layer()).name;
            info!("Layers changed, on top: {}", name);
            svc.display.show_text(name)?;
        }

        svc.keyboard.flush();
        // After the report: a profile switch releases the keys on the old host and drops it.
        while let Some(command) = svc.keymap.take_bt_command() {
            svc.run(Command::Bt(command))?;
        }
        svc.poll_combos()?;
        svc.update_status()?;
        svc.update_firmware_state()?;

        if all_up && svc.keymap.is_idle() {
            config::apply_changes(&mut svc)?;
            // A new config may have another layout on top.
            svc.update_status()?;
            power::wait_for_activity(&notification, &mut svc, &mut joystick)?;
        } else {
            delay.delay_ms(DELAY_MS);
        }
    }
}
