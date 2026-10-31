//! CLI commands.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use desktop::firmware::{self, Firmware, FirmwareProgress};
use desktop::link::Options;
use desktop::session::{Opened, Session, WriteProgress};
use desktop::{config_file, connect, transport};
use protocol::hid;
use protocol::config::{self, Config};

/// `kb info`.
pub async fn info(options: &Options) -> Result<()> {
    let mut client = connect(options).await?;
    let versions = client.versions().await?;
    let build = firmware::hex(&versions.build_id);
    match firmware::bundled_firmware() {
        Some(bundled) => println!(
            "Firmware version:  {} build {build} (bundled: {} build {})",
            versions.firmware_version,
            bundled.version,
            firmware::hex(&bundled.build_id)
        ),
        None => println!("Firmware version:  {} build {build}", versions.firmware_version),
    }
    println!("Board:             {} ({})", versions.board, firmware::device_id_text(&versions.device_id));
    println!("Protocol version:  {} (app: {})", versions.protocol_version, hid::PROTOCOL_VERSION);
    println!("Update version:    {} (app: {})", versions.update_version, hid::UPDATE_VERSION);
    // The rest is in the info, which only a firmware of the app protocol sends.
    let session = match Session::open(client, versions).await? {
        Opened::Session(session) => session,
        Opened::Incompatible { reason, .. } => return Err(reason),
    };
    let info = session.info();
    println!("Config version:    {} (app: {})", info.config_version, config::CONFIG_VERSION);
    println!("Keys:              {}x{}", info.width, info.height);
    println!("Max layouts:       {}", info.max_layouts);
    println!("Max combos:        {}", info.max_combos);
    session.close().await;
    Ok(())
}

/// `kb pull <file>`.
pub async fn pull(options: &Options, path: &Path) -> Result<()> {
    let mut session = Session::connect(connect(options).await?).await?;
    let config = session.read_config().await?;
    session.close().await;

    config_file::save(path, &config)?;
    println!("Saved {} ({} layouts, {} combos)", path.display(), config.layouts.len(), config.combos.len());
    Ok(())
}

/// `kb push <file>`.
pub async fn push(options: &Options, path: &Path) -> Result<()> {
    let config = load_valid(path)?;

    let mut session = Session::connect(connect(options).await?).await?;
    let len = session.write_config(&config, |progress| {
        if let WriteProgress::Sent { done, total } = progress
            && done == total
        {
            eprintln!("Sent {total} bytes, applying (release all keys)...");
        }
    })
    .await
    .with_context(|| path.display().to_string())?;
    session.close().await;

    println!("Config applied ({} layouts, {} combos, {len} bytes)", config.layouts.len(), config.combos.len());
    Ok(())
}

/// `kb reset`.
pub async fn reset(options: &Options) -> Result<()> {
    let mut session = Session::connect(connect(options).await?).await?;
    session.reset_config().await?;
    session.close().await;
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
            let firmware = firmware::bundled_firmware()
                .context("this build has no bundled firmware (see scripts/bundle.sh), give an image file")?;
            (firmware.clone(), "the bundled firmware".to_string())
        }
    };
    let version = &firmware.version;

    let mut client = connect(options).await?;
    let versions = client.versions().await?;
    eprintln!("Updating firmware {} to {version} ({} bytes)", versions.firmware_version, firmware.image.len());
    let start = Instant::now();
    let mut last_percent = None;
    let result = firmware::update_firmware(&mut client, &versions, &firmware, |progress| match progress {
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
            client.close().await;
            return Err(error.context(source));
        }
    };
    // The keyboard restarts, the session is gone with it.
    drop(client);
    eprintln!("Firmware sent ({sent} bytes compressed) in {:.0?}, waiting for the keyboard to restart...", start.elapsed());

    let (client, versions) = firmware::reconnect_after_update(versions.device_id, async || connect(options).await).await?;
    client.close().await;
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
