//! `kb-gui`: keyboard config editor with Bluetooth LE sync.
//!
//! Edits the same TOML files as the `kb` CLI: layouts on a key grid, special combos,
//! pull / push / reset over the keyboard config service.

// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod device;
mod editing;
mod labels;
mod views;

use clap::Parser;
use eframe::egui;

#[derive(Parser)]
#[command(about = "Keyboard config editor with Bluetooth LE sync")]
struct Args {
    /// Opens the editor with the example config right away, without connecting to the keyboard.
    #[arg(long)]
    offline: bool,
}

fn main() -> eframe::Result {
    let args = Args::parse();
    // WSLg's Wayland windows have no frame and can't be resized by the app, its X11 ones can.
    if std::env::var_os("WSL_DISTRO_NAME").is_some() {
        // SAFETY: no other threads yet; winit reads the variable when the event loop starts.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Keyboard config")
            .with_inner_size([1180.0, 640.0])
            .with_min_inner_size(app::MIN_SIZE),
        ..Default::default()
    };
    eframe::run_native("kb-gui", options, Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx, args.offline)))))
}
