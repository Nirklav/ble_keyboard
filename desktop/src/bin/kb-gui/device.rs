//! Keyboard connection off the UI thread.
//!
//! [`DeviceWorker`] runs a thread with a single-threaded tokio runtime that keeps the [`Device`]
//! and runs [`Command`]s one at a time (the device can't be shared between threads anyway). Every command ends with exactly one final [`Event`]
//! ([`Event::is_final`]), possibly after some [`Event::Progress`]. The UI polls the events
//! every frame; the worker requests a repaint when it sends one.
//!
//! While connected, the worker pings the keyboard every [`ble::PING_INTERVAL`] so it doesn't
//! deep sleep; [`Command::Disconnect`] and dropping the worker end the session.
//!
//! A fake worker ([`DeviceWorker::spawn`] with `fake`) answers like a keyboard without Bluetooth,
//! to look at the UI in every connection state.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use desktop::EXAMPLE_CONFIG;
use desktop::config_file;
use desktop::ops::{self, WriteProgress};
use desktop::transport::{Device, Options};
use eframe::egui;
use protocol::ble::{self, Info, Request};
use protocol::config::Config;
use tokio::sync::mpsc as async_mpsc;
use tokio::time::{MissedTickBehavior, interval, sleep};

/// How long dropping the worker waits for it to end the session.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// Request to the worker.
#[derive(Debug)]
pub enum Command {
    /// Only connects, the edited config stays as it is.
    Connect,
    /// Reads the config from the keyboard.
    Pull,
    /// Writes the config to the keyboard.
    Push(Box<Config>),
    /// Restores the factory config.
    Reset,
    Disconnect,
}

/// Message from the worker.
#[derive(Debug)]
pub enum Event {
    /// Status text of the running command.
    Progress(String),
    /// Connected to the keyboard (sent before the command continues).
    Connected(Info),
    /// [`Command::Connect`] or [`Command::Reset`] is done, with a status text.
    Done(String),
    Disconnected,
    Pulled(Box<Config>),
    Pushed {
        bytes: usize,
    },
    /// The command failed; the connection is dropped and made again by the next command.
    Failed(String),
}

impl Event {
    /// Whether the command is finished.
    pub fn is_final(&self) -> bool {
        !matches!(self, Event::Progress(_) | Event::Connected(_))
    }
}

/// Handle of the worker thread; dropping it stops the thread after the running command
/// and waits a bit for it to end the session.
pub struct DeviceWorker {
    /// `None` only while dropping.
    commands: Option<async_mpsc::UnboundedSender<(Options, Command)>>,
    events: mpsc::Receiver<Event>,
    /// Closed when the thread is done.
    done: mpsc::Receiver<()>,
}

impl DeviceWorker {
    /// `fake` runs [`run_fake`] instead of talking to a real keyboard.
    pub fn spawn(ctx: egui::Context, fake: bool) -> Result<DeviceWorker> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("can't start the Bluetooth worker")?;
        let (commands, command_rx) = async_mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::channel();
        let sender = EventSender { events: event_tx, ctx };
        let (done_tx, done) = mpsc::channel::<()>();
        thread::Builder::new()
            .name("kb-ble".into())
            .spawn(move || {
                if fake {
                    runtime.block_on(run_fake(command_rx, sender));
                } else {
                    runtime.block_on(run(command_rx, sender));
                }
                drop(done_tx);
            })
            .context("can't start the Bluetooth worker")?;
        Ok(DeviceWorker { commands: Some(commands), events, done })
    }

    pub fn send(&self, options: Options, command: Command) {
        // Fails only if the worker task is gone (it panicked), nothing to do then.
        if let Some(commands) = &self.commands {
            let _ = commands.send((options, command));
        }
    }

    /// Events received since the last call.
    pub fn poll(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}

impl Drop for DeviceWorker {
    fn drop(&mut self) {
        // Closing the channel stops the worker, which ends the session first.
        self.commands = None;
        let _ = self.done.recv_timeout(CLOSE_TIMEOUT);
    }
}

struct EventSender {
    events: mpsc::Sender<Event>,
    ctx: egui::Context,
}

impl EventSender {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }

    fn progress(&self, text: impl Into<String>) {
        self.send(Event::Progress(text.into()));
    }
}

struct Connection {
    device: Device,
    info: Info,
}

async fn run(mut commands: async_mpsc::UnboundedReceiver<(Options, Command)>, events: EventSender) {
    let mut connection: Option<Connection> = None;
    let mut ping = interval(ble::PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some((options, command)) = command else { break };
                let event = match execute(&mut connection, &options, command, &events).await {
                    Ok(event) => event,
                    Err(error) => {
                        // The device may be in an unknown state (a timed out request, a late response).
                        connection = None;
                        Event::Failed(format!("{error:#}"))
                    }
                };
                events.send(event);
                // Any request keeps the session alive too.
                ping.reset();
            }
            _ = ping.tick(), if connection.is_some() => {
                let Some(open) = &mut connection else { continue };
                if let Err(error) = open.device.request_ok(&Request::Ping).await {
                    connection = None;
                    events.send(Event::Failed(format!("lost the keyboard: {error:#}")));
                }
            }
        }
    }
    if let Some(connection) = connection {
        connection.device.close().await;
    }
}

async fn execute(
    connection: &mut Option<Connection>,
    options: &Options,
    command: Command,
    events: &EventSender,
) -> Result<Event> {
    if matches!(command, Command::Disconnect) {
        if let Some(old) = connection.take() {
            old.device.close().await;
        }
        return Ok(Event::Disconnected);
    }
    let Connection { device, info } = match connection {
        Some(connection) => connection,
        None => connection.insert(connect(options, events).await?),
    };

    Ok(match command {
        Command::Connect => Event::Done("Connected to the keyboard".to_string()),
        Command::Pull => {
            events.progress("Reading the config...");
            Event::Pulled(Box::new(ops::read_config(device, info).await?))
        }
        Command::Push(config) => {
            events.progress("Sending the config...");
            let bytes = ops::write_config(device, info, &config, |progress| match progress {
                WriteProgress::Sent { done, total } => events.progress(format!("Sent {done} of {total} bytes...")),
                WriteProgress::Applying => events.progress("Applying, release all keys..."),
            })
            .await?;
            Event::Pushed { bytes }
        }
        Command::Reset => {
            events.progress("Restoring the factory config...");
            ops::reset_config(device, info).await?;
            Event::Done("Factory config restored on the keyboard, Read to edit it".to_string())
        }
        Command::Disconnect => unreachable!("handled above"),
    })
}

async fn connect(options: &Options, events: &EventSender) -> Result<Connection> {
    events.progress(format!("Looking for \"{}\"...", options.name));
    let device = Device::connect(options).await?;
    events.progress("Reading the keyboard info...");
    let info = device.info().await?;
    events.send(Event::Connected(info.clone()));
    ops::check_protocol(&info)?;
    Ok(Connection { device, info })
}

/// Fake keyboard: connects, reads, writes and resets like a real one, with delays and progress,
/// keeping the written config; starts with the example config.
async fn run_fake(mut commands: async_mpsc::UnboundedReceiver<(Options, Command)>, events: EventSender) {
    let factory = config_file::parse(EXAMPLE_CONFIG).expect("the example config is valid");
    let mut stored = factory.clone();
    let mut connected = false;
    while let Some((options, command)) = commands.recv().await {
        if matches!(command, Command::Disconnect) {
            connected = false;
            events.send(Event::Disconnected);
            continue;
        }
        if !connected {
            events.progress(format!("Looking for \"{}\"...", options.name));
            sleep(Duration::from_millis(1500)).await;
            events.progress("Reading the keyboard info...");
            sleep(Duration::from_millis(300)).await;
            events.send(Event::Connected(fake_info()));
            connected = true;
        }
        let event = match command {
            Command::Connect => Event::Done("Connected to the keyboard".to_string()),
            Command::Pull => {
                events.progress("Reading the config...");
                sleep(Duration::from_millis(600)).await;
                Event::Pulled(Box::new(stored.clone()))
            }
            Command::Push(config) => {
                let total = config.to_bytes().len();
                for done in (0..total).step_by(128).skip(1).chain([total]) {
                    events.progress(format!("Sent {done} of {total} bytes..."));
                    sleep(Duration::from_millis(80)).await;
                }
                events.progress("Applying, release all keys...");
                sleep(Duration::from_millis(500)).await;
                stored = *config;
                Event::Pushed { bytes: total }
            }
            Command::Reset => {
                events.progress("Restoring the factory config...");
                sleep(Duration::from_millis(800)).await;
                stored = factory.clone();
                Event::Done("Factory config restored on the keyboard, Read to edit it".to_string())
            }
            Command::Disconnect => unreachable!("handled above"),
        };
        events.send(event);
    }
}

/// Info of the fake keyboard: the current versions and the default 10x5 grid.
fn fake_info() -> Info {
    Info {
        protocol_version: ble::PROTOCOL_VERSION,
        config_version: protocol::config::CONFIG_VERSION,
        width: 10,
        height: 5,
        max_layouts: protocol::config::MAX_LAYOUTS as u8,
        max_combos: protocol::config::MAX_COMBOS as u8,
        firmware_version: "0.0.0-fake".to_string(),
    }
}
