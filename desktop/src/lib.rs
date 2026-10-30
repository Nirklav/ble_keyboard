//! Keyboard config over HID (hidapi natively, WebHID in the browser): shared code of the `kb` CLI
//! and the `kb-gui` app.

pub mod compat;
pub mod config_file;
pub mod firmware;
pub mod link;
pub mod session;
pub mod time;
#[cfg(not(target_arch = "wasm32"))]
pub mod transport;
#[cfg(target_arch = "wasm32")]
pub mod webhid;

/// Keyboard connection of this build.
#[cfg(not(target_arch = "wasm32"))]
pub use transport::Device;
/// Keyboard connection of this build.
#[cfg(target_arch = "wasm32")]
pub use webhid::Device;

/// Client of the connected keyboard, see `Device::connect`.
pub async fn connect(options: &link::Options) -> anyhow::Result<link::Client<Device>> {
    Ok(link::Client::new(Device::connect(options).await?))
}

/// Factory config of the keyboard as TOML, see `examples/config.toml`.
pub const EXAMPLE_CONFIG: &str = include_str!("../examples/config.toml");

/// Keyboard firmware image bundled with the app at build time (`KB_FIRMWARE`, see `build.rs`).
#[cfg(bundled_firmware)]
pub const BUNDLED_FIRMWARE: Option<&[u8]> = Some(include_bytes!(concat!(env!("OUT_DIR"), "/firmware.bin")));
/// Keyboard firmware image bundled with the app at build time (`KB_FIRMWARE`, see `build.rs`).
#[cfg(not(bundled_firmware))]
pub const BUNDLED_FIRMWARE: Option<&[u8]> = None;
