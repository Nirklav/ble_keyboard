# Bump versions with changes

Bump the crate version of the firmware (`keyboard/Cargo.toml`) and of the desktop app (`desktop/Cargo.toml`) together with every change to that crate. Started at 0.2.0 on 2026-10-29.

**Why:** the firmware version (CARGO_PKG_VERSION, in the app image and in `Versions`/`Info`) is the only way the app tells the bundled firmware from the one on the keyboard (Update vs Reinstall in kb-gui); with a constant 0.1.0 every build looked the same.

**How to apply:** when a change touches `keyboard/` or `desktop/`, bump that crate's version in the same change (and mention it). Protocol changes also follow `PROTOCOL_VERSION` / `UPDATE_VERSION` rules in `protocol/src/hid.rs`, see [web-config-via-webhid](web-config-via-webhid.md).
