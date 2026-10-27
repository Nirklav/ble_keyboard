//! Config files: the native file dialogs, or in the browser an upload (`<input type=file>`) and a download.
//!
//! Opening is asynchronous in the browser, so its result always comes through [`Files::poll`].

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::Result;
use desktop::config_file;
use eframe::egui;
use protocol::config::Config;

/// Config read from a file with the file, or why it couldn't be read.
pub type Opened = Result<(Config, PathBuf), String>;

pub struct Files {
    sender: mpsc::Sender<Opened>,
    opened: mpsc::Receiver<Opened>,
}

impl Files {
    pub fn new() -> Files {
        let (sender, opened) = mpsc::channel();
        Files { sender, opened }
    }

    /// File opened since the last call.
    pub fn poll(&self) -> Option<Opened> {
        self.opened.try_recv().ok()
    }

    /// Asks for a config file to open, starting in the folder of `current`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(&self, ctx: &egui::Context, current: Option<&Path>) {
        let Some(path) = dialog(current).pick_file() else { return };
        let opened = config_file::load(&path).map(|config| (config, path)).map_err(|e| format!("{e:#}"));
        let _ = self.sender.send(opened);
        // Polled on the next frame, which nothing else may ask for.
        ctx.request_repaint();
    }

    /// Asks for a config file to upload; `ctx` is repainted once it's read.
    #[cfg(target_arch = "wasm32")]
    pub fn open(&self, ctx: &egui::Context, _current: Option<&Path>) {
        if let Err(error) = web::pick(ctx.clone(), self.sender.clone()) {
            let _ = self.sender.send(Err(format!("can't open the file dialog: {error:?}")));
        }
    }

    /// Saves `config` to `current`, or (also with `ask`) to a picked file.
    /// Returns the saved file, `None` if cancelled.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, current: Option<&Path>, config: &Config, ask: bool) -> Result<Option<PathBuf>> {
        let path = match current {
            Some(path) if !ask => path.to_path_buf(),
            _ => {
                let name = current.and_then(Path::file_name).map_or("config.toml".into(), |n| n.to_string_lossy());
                match dialog(current).set_file_name(name).save_file() {
                    Some(path) => path,
                    None => return Ok(None),
                }
            }
        };
        config_file::save(&path, config)?;
        Ok(Some(path))
    }

    /// Downloads `config` as the file name of `current`; the browser decides where it goes.
    #[cfg(target_arch = "wasm32")]
    pub fn save(&self, current: Option<&Path>, config: &Config, _ask: bool) -> Result<Option<PathBuf>> {
        let name = current.and_then(Path::file_name).map_or("config.toml".into(), |n| n.to_string_lossy().into_owned());
        web::download(&name, &config_file::format(config)?).map_err(|e| anyhow::anyhow!("can't download: {e:?}"))?;
        Ok(Some(PathBuf::from(name)))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn dialog(current: Option<&Path>) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new().add_filter("TOML config", &["toml"]).add_filter("All files", &["*"]);
    match current.and_then(Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => dialog.set_directory(dir),
        _ => dialog,
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::PathBuf;
    use std::sync::mpsc;

    use desktop::config_file;
    use eframe::egui;
    use wasm_bindgen::prelude::*;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, HtmlInputElement, Url};

    use super::Opened;

    fn document() -> Result<web_sys::Document, JsValue> {
        web_sys::window().and_then(|w| w.document()).ok_or_else(|| "no document".into())
    }

    /// Clicks a file input (within the activation of the click on the app), the chosen file is
    /// read and sent; nothing if the dialog is cancelled.
    pub fn pick(ctx: egui::Context, sender: mpsc::Sender<Opened>) -> Result<(), JsValue> {
        let input: HtmlInputElement = document()?.create_element("input")?.dyn_into()?;
        input.set_type("file");
        input.set_accept(".toml,text/plain");
        let target = input.clone();
        let on_change = Closure::once_into_js(move || {
            let Some(file) = target.files().and_then(|files| files.get(0)) else { return };
            wasm_bindgen_futures::spawn_local(async move {
                let name = file.name();
                let opened = match file.text().await.map(|text| text.as_string()) {
                    Ok(Some(text)) => config_file::parse(&text)
                        .map(|config| (config, PathBuf::from(&name)))
                        .map_err(|e| format!("can't parse {name}: {e:#}")),
                    _ => Err(format!("can't read {name}")),
                };
                let _ = sender.send(opened);
                ctx.request_repaint();
            });
        });
        input.add_event_listener_with_callback("change", on_change.unchecked_ref())?;
        input.click();
        Ok(())
    }

    pub fn download(name: &str, text: &str) -> Result<(), JsValue> {
        let options = BlobPropertyBag::new();
        options.set_type("text/plain");
        let blob = Blob::new_with_str_sequence_and_options(&js_sys::Array::of1(&text.into()), &options)?;
        let url = Url::create_object_url_with_blob(&blob)?;
        let link: HtmlAnchorElement = document()?.create_element("a")?.dyn_into()?;
        link.set_href(&url);
        link.set_download(name);
        link.click();
        // Some browsers start the download after the click returns, the URL must outlive it.
        wasm_bindgen_futures::spawn_local(async move {
            desktop::time::sleep(std::time::Duration::from_secs(10)).await;
            let _ = Url::revoke_object_url(&url);
        });
        Ok(())
    }
}
