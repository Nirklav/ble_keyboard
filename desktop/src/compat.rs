//! What the app can do with a keyboard: the versions of its firmware, the limits for a config.

use anyhow::{Result, ensure};
use protocol::board;
use protocol::config::{self, Config};
use protocol::hid::{self, Info, Versions};

use crate::config_file;
use crate::firmware::Firmware;

/// Checks what blocks a write: validity for the keyboard size, the keyboard limits, the encoded size.
/// `info` is the connected keyboard, if any; without one the [`board`] size.
pub fn check_config(config: &Config, info: Option<&Info>) -> Result<(), String> {
    let (width, height) = info.map_or((board::WIDTH, board::HEIGHT), |info| (info.width, info.height));
    config.validate_for(width, height).map_err(|e| config_file::describe(&e, config))?;
    if let Some(info) = info {
        if config.layouts.len() > info.max_layouts as usize {
            return Err(format!("the keyboard supports at most {} layouts", info.max_layouts));
        }
        if config.combos.len() > info.max_combos as usize {
            return Err(format!("the keyboard supports at most {} combos", info.max_combos));
        }
    }
    let size = config.to_bytes().len();
    if size > hid::MAX_CONFIG_SIZE {
        return Err(format!("encoded config is {size} bytes, max is {}", hid::MAX_CONFIG_SIZE));
    }
    Ok(())
}

/// Fails if the firmware speaks a different protocol version.
pub fn check_protocol(protocol_version: u16) -> Result<()> {
    ensure!(
        protocol_version == hid::PROTOCOL_VERSION,
        "keyboard protocol version {protocol_version} differs from the app one {}, update the firmware or the app",
        hid::PROTOCOL_VERSION
    );
    Ok(())
}

/// Fails if the app can't read the keyboard config version (a newer firmware, or one older than the upgrades go).
pub fn check_config_version(info: &Info) -> Result<()> {
    ensure!(
        (config::OLDEST_CONFIG_VERSION..=config::CONFIG_VERSION).contains(&info.config_version),
        "keyboard config version {} is not supported by the app (versions {}..={}), update the firmware or the app",
        info.config_version,
        config::OLDEST_CONFIG_VERSION,
        config::CONFIG_VERSION
    );
    Ok(())
}

/// Fails if the app can't install `firmware` on the keyboard: its update part of the protocol is another one,
/// the image is for another board or doesn't fit the update slot.
pub fn check_update(versions: &Versions, firmware: &Firmware) -> Result<()> {
    ensure!(
        versions.update_version != 0,
        "keyboard firmware {} has no update support yet, flash it over USB once",
        versions.firmware_version
    );
    ensure!(
        versions.update_version == hid::UPDATE_VERSION,
        "keyboard firmware {} updates with protocol version {}, the app with {}: flash it over USB",
        versions.firmware_version,
        versions.update_version,
        hid::UPDATE_VERSION
    );
    ensure!(
        firmware.board == versions.board,
        "the firmware is for board `{}`, the keyboard is `{}`",
        firmware.board,
        versions.board
    );
    ensure!(
        firmware.image.len() <= versions.slot_size as usize,
        "the firmware is {} bytes, the keyboard has room for {}",
        firmware.image.len(),
        versions.slot_size
    );
    Ok(())
}
