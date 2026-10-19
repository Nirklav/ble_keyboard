//! Keyboard config over Bluetooth LE: shared code of the `kb` CLI and the `kb-gui` app.

pub mod config_file;
pub mod ops;
pub mod transport;

/// Factory config of the keyboard as TOML, see `examples/config.toml`.
pub const EXAMPLE_CONFIG: &str = include_str!("../examples/config.toml");
