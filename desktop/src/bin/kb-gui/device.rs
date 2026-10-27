//! Keyboard connection off the UI thread.
//!
//! [`DeviceWorker`] runs a task that keeps the [`Device`] and runs [`Command`]s one at a time: natively
//! on a thread with a single-threaded tokio runtime (the device can't be shared between threads anyway),
//! in the browser on its event loop. Every command ends with exactly one final [`Event`]
//! ([`Event::is_final`]), possibly after some [`Event::Progress`]. The UI polls the events
//! every frame; the worker requests a repaint when it sends one.
//!
//! While connected, the worker pings the keyboard every [`hid::PING_INTERVAL`] so it doesn't
//! deep sleep; [`Command::Disconnect`] and dropping the worker end the session.
//!
//! A fake worker ([`DeviceWorker::spawn`] with `fake`) answers like a keyboard without a real one,
//! to look at the UI in every connection state.

use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use desktop::link::{Link, Options};
use desktop::ops::{self, WriteProgress};
use desktop::time::{sleep, timeout};
use desktop::{Device, EXAMPLE_CONFIG, config_file};
use eframe::egui;
use futures::StreamExt;
use futures::channel::mpsc as async_mpsc;
use protocol::hid::{self, Info, Request};
use protocol::config::Config;

/// How long dropping the worker waits for it to end the session.
#[cfg(not(target_arch = "wasm32"))]
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

type Commands = async_mpsc::UnboundedReceiver<(Options, Command)>;

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

/// Handle of the worker; dropping it stops the worker after the running command and (natively)
/// waits a bit for it to end the session.
pub struct DeviceWorker {
    /// `None` only while dropping.
    commands: Option<async_mpsc::UnboundedSender<(Options, Command)>>,
    events: mpsc::Receiver<Event>,
    /// Closed when the thread is done.
    #[cfg(not(target_arch = "wasm32"))]
    done: mpsc::Receiver<()>,
}

impl DeviceWorker {
    /// `fake` runs [`run_fake`] instead of talking to a real keyboard.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn(ctx: egui::Context, fake: bool) -> Result<DeviceWorker> {
        use anyhow::Context;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("can't start the keyboard worker")?;
        let (commands, command_rx) = async_mpsc::unbounded();
        let (event_tx, events) = mpsc::channel();
        let sender = EventSender { events: event_tx, ctx };
        let (done_tx, done) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("kb-device".into())
            .spawn(move || {
                runtime.block_on(work(command_rx, sender, fake));
                drop(done_tx);
            })
            .context("can't start the keyboard worker")?;
        Ok(DeviceWorker { commands: Some(commands), events, done })
    }

    #[cfg(target_arch = "wasm32")]
    pub fn spawn(ctx: egui::Context, fake: bool) -> Result<DeviceWorker> {
        let (commands, command_rx) = async_mpsc::unbounded();
        let (event_tx, events) = mpsc::channel();
        let sender = EventSender { events: event_tx, ctx };
        wasm_bindgen_futures::spawn_local(work(command_rx, sender, fake));
        Ok(DeviceWorker { commands: Some(commands), events })
    }

    pub fn send(&self, options: Options, command: Command) {
        // Fails only if the worker task is gone (it panicked), nothing to do then.
        if let Some(commands) = &self.commands {
            let _ = commands.unbounded_send((options, command));
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
        #[cfg(not(target_arch = "wasm32"))]
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

async fn work(commands: Commands, events: EventSender, fake: bool) {
    if fake {
        run_fake(commands, events).await;
    } else {
        run(commands, events).await;
    }
}

async fn run(mut commands: Commands, events: EventSender) {
    let mut connection: Option<Connection> = None;
    loop {
        // Pings when no command came for an interval: any request keeps the session alive too.
        let command = match &mut connection {
            Some(open) => match timeout(hid::PING_INTERVAL, commands.next()).await {
                Some(command) => command,
                None => {
                    if let Err(error) = open.device.request_ok(&Request::Ping).await {
                        connection = None;
                        events.send(Event::Failed(format!("lost the keyboard: {error:#}")));
                    }
                    continue;
                }
            },
            None => commands.next().await,
        };
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
    events.progress("Connecting to the keyboard...");
    let mut device = Device::connect(options).await?;
    events.progress("Reading the keyboard info...");
    let info = device.info().await?;
    events.send(Event::Connected(info.clone()));
    ops::check_protocol(&info)?;
    Ok(Connection { device, info })
}

/// Fake keyboard: connects, reads, writes and resets like a real one, with delays and progress,
/// keeping the written config; starts with the example config.
async fn run_fake(mut commands: Commands, events: EventSender) {
    let factory = config_file::parse(EXAMPLE_CONFIG).expect("the example config is valid");
    let mut stored = factory.clone();
    let mut connected = false;
    while let Some((_, command)) = commands.next().await {
        if matches!(command, Command::Disconnect) {
            connected = false;
            events.send(Event::Disconnected);
            continue;
        }
        if !connected {
            events.progress("Connecting to the keyboard...");
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
        protocol_version: hid::PROTOCOL_VERSION,
        config_version: protocol::config::CONFIG_VERSION,
        width: 10,
        height: 5,
        max_layouts: protocol::config::MAX_LAYOUTS as u8,
        max_combos: protocol::config::MAX_COMBOS as u8,
        firmware_version: "0.0.0-fake".to_string(),
    }
}
