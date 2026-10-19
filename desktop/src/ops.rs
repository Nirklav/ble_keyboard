//! Config operations on a connected keyboard, shared by the CLI and the GUI.

use anyhow::{Context, Result, anyhow, bail, ensure};
use protocol::ble::{self, Info, Request, Response};
use protocol::config::{self, Config};
use protocol::crc::crc32;

use crate::config_file;
use crate::transport::Device;

/// Progress of [`write_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteProgress {
    /// `done` of `total` bytes are sent.
    Sent { done: usize, total: usize },
    /// All bytes are sent, the keyboard is applying the config (waits until all keys are released).
    Applying,
}

/// Fails if the firmware speaks a different protocol version.
pub fn check_protocol(info: &Info) -> Result<()> {
    ensure!(
        info.protocol_version == ble::PROTOCOL_VERSION,
        "keyboard protocol version {} differs from the app one {}, update the firmware or the app",
        info.protocol_version,
        ble::PROTOCOL_VERSION
    );
    Ok(())
}

/// Reads, decodes and validates the keyboard config.
pub async fn read_config(device: &mut Device, info: &Info) -> Result<Config> {
    check_protocol(info)?;
    let bytes = read_config_bytes(device).await?;
    let config = Config::from_bytes(&bytes).context("can't decode the keyboard config")?;
    config
        .validate_for(info.width, info.height)
        .map_err(|e| anyhow!("keyboard config is invalid: {}", config_file::describe(&e, &config)))?;
    Ok(config)
}

async fn read_config_bytes(device: &mut Device) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut expected_len = None;
    loop {
        let offset = bytes.len() as u32;
        let response = device.request(&Request::ReadConfig { offset }).await?;
        let Response::ConfigChunk { total_len, offset: chunk_offset, data } = response else {
            bail!("unexpected response to ReadConfig: {response:?}");
        };
        ensure!(chunk_offset == offset, "asked for offset {offset}, got {chunk_offset}");
        ensure!(total_len as usize <= ble::MAX_CONFIG_SIZE, "config of {total_len} bytes is too large");
        if *expected_len.get_or_insert(total_len) != total_len {
            bail!("config changed while reading, try again");
        }

        bytes.extend_from_slice(&data);
        ensure!(bytes.len() <= total_len as usize, "keyboard sent more than {total_len} bytes");
        if bytes.len() == total_len as usize {
            return Ok(bytes);
        }
        ensure!(!data.is_empty(), "keyboard sent an empty chunk at {offset} of {total_len}");
    }
}

/// Validates `config` against the keyboard and writes it. Returns the number of bytes sent.
pub async fn write_config(
    device: &mut Device,
    info: &Info,
    config: &Config,
    mut progress: impl FnMut(WriteProgress),
) -> Result<usize> {
    check_protocol(info)?;
    ensure!(
        info.config_version == config::CONFIG_VERSION,
        "keyboard config version {} differs from the app one {}, update the firmware or the app",
        info.config_version,
        config::CONFIG_VERSION
    );
    config.validate_for(info.width, info.height).map_err(|e| anyhow!("{}", config_file::describe(&e, config)))?;
    ensure!(
        config.layouts.len() <= info.max_layouts as usize,
        "keyboard supports at most {} layouts",
        info.max_layouts
    );
    ensure!(config.combos.len() <= info.max_combos as usize, "keyboard supports at most {} combos", info.max_combos);

    let bytes = config.to_bytes();
    ensure!(bytes.len() <= ble::MAX_CONFIG_SIZE, "config is {} bytes, max is {}", bytes.len(), ble::MAX_CONFIG_SIZE);
    let chunk_size = device.chunk_size();
    ensure!(chunk_size > 0, "MTU {} is too small", device.mtu());

    let total = bytes.len();
    device.request_ok(&Request::WriteBegin { len: total as u32, crc32: crc32(&bytes) }).await?;
    for (offset, chunk) in ble::chunks(&bytes, chunk_size) {
        device.request_ok(&Request::WriteChunk { offset, data: chunk.to_vec() }).await?;
        progress(WriteProgress::Sent { done: offset as usize + chunk.len(), total });
    }
    progress(WriteProgress::Applying);
    device.request_ok(&Request::WriteCommit).await?;
    Ok(total)
}

/// Restores the factory config on the keyboard.
pub async fn reset_config(device: &mut Device, info: &Info) -> Result<()> {
    check_protocol(info)?;
    device.request_ok(&Request::ResetToDefault).await
}
