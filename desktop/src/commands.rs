//! CLI commands.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use desktop::config_file;
use desktop::ops::{self, WriteProgress};
use desktop::link::{Link, Options};
use desktop::transport::{self, Device};
use protocol::hid;
use protocol::config::{self, Config};

/// `kb info`.
pub async fn info(options: &Options) -> Result<()> {
    let mut device = Device::connect(options).await?;
    let info = device.info().await?;
    println!("Firmware version:  {}", info.firmware_version);
    println!("Protocol version:  {} (app: {})", info.protocol_version, hid::PROTOCOL_VERSION);
    println!("Config version:    {} (app: {})", info.config_version, config::CONFIG_VERSION);
    println!("Keys:              {}x{}", info.width, info.height);
    println!("Max layouts:       {}", info.max_layouts);
    println!("Max combos:        {}", info.max_combos);
    device.close().await;
    Ok(())
}

/// `kb pull <file>`.
pub async fn pull(options: &Options, path: &Path) -> Result<()> {
    let mut device = Device::connect(options).await?;
    let info = device.info().await?;
    let config = ops::read_config(&mut device, &info).await?;
    device.close().await;

    config_file::save(path, &config)?;
    println!("Saved {} ({} layouts, {} combos)", path.display(), config.layouts.len(), config.combos.len());
    Ok(())
}

/// `kb push <file>`.
pub async fn push(options: &Options, path: &Path) -> Result<()> {
    let config = load_valid(path)?;

    let mut device = Device::connect(options).await?;
    let info = device.info().await?;
    let len = ops::write_config(&mut device, &info, &config, |progress| {
        if let WriteProgress::Sent { done, total } = progress
            && done == total
        {
            eprintln!("Sent {total} bytes, applying (release all keys)...");
        }
    })
    .await
    .with_context(|| path.display().to_string())?;
    device.close().await;

    println!("Config applied ({} layouts, {} combos, {len} bytes)", config.layouts.len(), config.combos.len());
    Ok(())
}

/// `kb reset`.
pub async fn reset(options: &Options) -> Result<()> {
    let mut device = Device::connect(options).await?;
    let info = device.info().await?;
    ops::reset_config(&mut device, &info).await?;
    device.close().await;
    println!("Factory config restored");
    Ok(())
}

/// `kb check <file>`: offline validation.
pub fn check(path: &Path) -> Result<()> {
    let config = load_valid(path)?;
    let size = config.to_bytes().len();
    println!(
        "{}: OK, {}x{} keys, {} layouts ({}), {} combos, {size} bytes encoded",
        path.display(),
        config.width,
        config.height,
        config.layouts.len(),
        config.layouts.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", "),
        config.combos.len(),
    );
    if size > hid::MAX_CONFIG_SIZE {
        bail!("encoded config is larger than {} bytes", hid::MAX_CONFIG_SIZE);
    }
    Ok(())
}

fn load_valid(path: &Path) -> Result<Config> {
    let config = config_file::load(path)?;
    config.validate().map_err(|e| anyhow!("{}: {}", path.display(), config_file::describe(&e, &config)))?;
    Ok(config)
}

/// `kb udev-rule`.
pub fn udev_rule() {
    println!("{}", transport::UDEV_RULE);
    eprintln!();
    eprintln!("Install it (then reconnect the keyboard):");
    eprintln!("  kb udev-rule 2>/dev/null | sudo tee {}", transport::UDEV_RULE_FILE);
    eprintln!("  sudo udevadm control --reload-rules && sudo udevadm trigger");
}
