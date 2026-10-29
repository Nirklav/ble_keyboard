//! CLI commands.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use desktop::config_file;
use desktop::ops::{self, Firmware, FirmwareProgress, WriteProgress};
use desktop::link::{Link, Options};
use desktop::transport::{self, Device};
use protocol::hid;
use protocol::config::{self, Config};

/// `kb info`.
pub async fn info(options: &Options) -> Result<()> {
    let mut device = Device::connect(options).await?;
    let versions = device.versions().await?;
    let build = ops::hex(&versions.build_id);
    match ops::bundled_firmware() {
        Some(bundled) => println!(
            "Firmware version:  {} build {build} (bundled: {} build {})",
            versions.firmware_version,
            bundled.version,
            ops::hex(&bundled.build_id)
        ),
        None => println!("Firmware version:  {} build {build}", versions.firmware_version),
    }
    println!("Board:             {} ({})", versions.board, ops::device_id_text(&versions.device_id));
    println!("Protocol version:  {} (app: {})", versions.protocol_version, hid::PROTOCOL_VERSION);
    println!("Update version:    {} (app: {})", versions.update_version, hid::UPDATE_VERSION);
    // The rest is in the info, which only a firmware of the app protocol sends.
    if let Err(error) = ops::check_protocol(versions.protocol_version) {
        device.close().await;
        return Err(error);
    }
    // The versions are read and checked already, `ops::connect_info` would ask again.
    let info = device.info().await?;
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
    let info = ops::connect_info(&mut device).await?;
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
    let info = ops::connect_info(&mut device).await?;
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
    let info = ops::connect_info(&mut device).await?;
    ops::reset_config(&mut device, &info).await?;
    device.close().await;
    println!("Factory config restored");
    Ok(())
}

/// `kb update [file]`: the bundled firmware without a file.
pub async fn update(options: &Options, path: Option<&Path>) -> Result<()> {
    let file;
    let (firmware, source) = match path {
        Some(path) => {
            file = std::fs::read(path).with_context(|| format!("can't read {}", path.display()))?;
            let firmware = Firmware::parse(&file).with_context(|| path.display().to_string())?;
            (firmware, path.display().to_string())
        }
        None => {
            let firmware = ops::bundled_firmware()
                .context("this build has no bundled firmware (see scripts/bundle.sh), give an image file")?;
            (firmware.clone(), "the bundled firmware".to_string())
        }
    };
    let version = &firmware.version;

    let mut device = Device::connect(options).await?;
    let versions = device.versions().await?;
    eprintln!("Updating firmware {} to {version} ({} bytes)", versions.firmware_version, firmware.image.len());
    let start = Instant::now();
    let mut last_percent = None;
    let result = ops::update_firmware(&mut device, &versions, &firmware, |progress| match progress {
        FirmwareProgress::Sent { done, total } => {
            let percent = done * 100 / total;
            if last_percent != Some(percent) {
                last_percent = Some(percent);
                let rate = done as f64 / start.elapsed().as_secs_f64().max(0.001) / 1024.0;
                eprint!("\rSent {percent:3}% of {total} bytes, {rate:.1} KiB/s");
                let _ = std::io::stderr().flush();
            }
        }
        FirmwareProgress::Checking => {
            last_percent = None;
            eprintln!("\nChecking the image...");
        }
    })
    .await;
    // The error goes on its own line, not after the progress one.
    let sent = match result {
        Ok(sent) => sent,
        Err(error) => {
            // The error goes on its own line, not after the progress one.
            if last_percent.is_some() {
                eprintln!();
            }
            // Aborts the transfer, the keyboard may sleep again.
            device.close().await;
            return Err(error.context(source));
        }
    };
    // The keyboard restarts, the session is gone with it.
    drop(device);
    eprintln!("Firmware sent ({sent} bytes compressed) in {:.0?}, waiting for the keyboard to restart...", start.elapsed());

    let (device, versions) =
        ops::reconnect_after_update(versions.device_id, async || Device::connect(options).await).await?;
    device.close().await;
    firmware.check_running(&versions)?;
    println!("Firmware {version} is running and confirmed");
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
