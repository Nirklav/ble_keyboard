//! `kb-gui`: keyboard config editor that reads and writes the keyboard over HID.
//!
//! Edits the same TOML files as the `kb` CLI: layouts on a key grid, special combos,
//! pull / push / reset over the keyboard config reports (hidapi).
//!
//! Also builds for the browser (`trunk serve` in this crate, see index.html), talking to the keyboard
//! over WebHID; `?offline` and `?fake-keyboard` in the URL work like the flags.

// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod device;
mod editing;
mod files;
mod labels;
mod theme;
mod views;

#[cfg(not(target_arch = "wasm32"))]
use eframe::egui;

#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Parser)]
#[command(about = "Keyboard config editor")]
struct Args {
    /// Opens the editor with the example config right away, without connecting to the keyboard.
    #[arg(long)]
    offline: bool,
    /// Talks to a fake keyboard instead of a real one, to see the UI connected.
    #[arg(long)]
    fake_keyboard: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use clap::Parser;

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
    eframe::run_native(
        "kb-gui",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx, args.offline, args.fake_keyboard)))),
    )
}

// The edited config isn't saved: closing or reloading the page asks first.
#[cfg(target_arch = "wasm32")]
thread_local! {
    static UNSAVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let window = web_sys::window().expect("runs in a browser window");
    let query = window.location().search().unwrap_or_default();
    // `?offline`, also with a value (`?offline=1`).
    let flag = |name: &str| query.trim_start_matches('?').split('&').any(|param| param.split('=').next() == Some(name));
    let (offline, fake_keyboard) = (flag("offline"), flag("fake-keyboard"));

    let on_unload = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(|event: web_sys::BeforeUnloadEvent| {
        if UNSAVED.get() {
            event.prevent_default();
            event.set_return_value("The config has unsaved changes");
        }
    });
    let _ = window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref());
    on_unload.forget();

    let document = window.document().expect("the page has a document");
    let canvas = document
        .get_element_by_id("app")
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("index.html has the #app canvas");
    wasm_bindgen_futures::spawn_local(async move {
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| Ok(Box::new(app::App::new(&cc.egui_ctx, offline, fake_keyboard)))),
            )
            .await;
        if let Some(status) = document.get_element_by_id("status") {
            match result {
                Ok(()) => status.remove(),
                Err(error) => status.set_text_content(Some(&format!("The app failed to start: {error:?}"))),
            }
        }
    });
}
