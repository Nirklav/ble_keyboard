//! `kb`: reads and writes the keyboard config over HID (the keyboard connected over Bluetooth LE).

mod commands;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use desktop::link::Options;

#[derive(Debug, Parser)]
#[command(name = "kb", version, about = "Keyboard config over HID")]
struct Cli {
    /// Timeout in seconds for every request.
    #[arg(long, global = true, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Prints firmware and protocol versions, keyboard size and limits.
    Info,
    /// Reads the config from the keyboard into a TOML file.
    Pull {
        /// TOML file to write.
        file: PathBuf,
    },
    /// Validates a TOML config and writes it to the keyboard.
    Push {
        /// TOML config file.
        file: PathBuf,
    },
    /// Restores the factory config on the keyboard.
    Reset,
    /// Validates a TOML config without connecting to the keyboard.
    Check {
        /// TOML config file.
        file: PathBuf,
    },
    /// Prints the Linux udev rule that gives the user access to the keyboard, and how to install it.
    UdevRule,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = Options { timeout: Duration::from_secs(cli.timeout) };

    let result = match &cli.command {
        Command::Info => commands::info(&options).await,
        Command::Pull { file } => commands::pull(&options, file).await,
        Command::Push { file } => commands::push(&options, file).await,
        Command::Reset => commands::reset(&options).await,
        Command::Check { file } => commands::check(file),
        Command::UdevRule => {
            commands::udev_rule();
            Ok(())
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
