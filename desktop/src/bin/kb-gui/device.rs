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
//! A fake worker ([`DeviceWorker::spawn`] with `fake`) talks to a [`FakeDevice`] instead of a real keyboard,
//! to look at the UI in every connection state.

use std::cell::RefCell;
use std::rc::Rc;
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
use protocol::config::{Config, MAX_COMBOS, MAX_LAYOUTS};
use protocol::hid::{self, ConfigReceiver, Info, Request, Response};

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
    /// `fake` talks to a [`FakeDevice`] instead of a real keyboard.
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

struct Connection<D> {
    device: D,
    info: Info,
}

async fn work(commands: Commands, events: EventSender, fake: bool) {
    if fake {
        let factory: Rc<[u8]> = config_file::parse(EXAMPLE_CONFIG).expect("the example config is valid").to_bytes().into();
        let stored = Rc::new(RefCell::new(factory.to_vec()));
        let (width, height) = ops::DEFAULT_SIZE;
        let connect = async |_: &Options| {
            sleep(Duration::from_millis(1000)).await;
            Ok(FakeDevice { factory: factory.clone(), stored: stored.clone(), receiver: ConfigReceiver::new(width, height) })
        };
        run(commands, events, connect).await;
    } else {
        run(commands, events, Device::connect).await;
    }
}

async fn run<D: Link>(mut commands: Commands, events: EventSender, mut connect: impl AsyncFnMut(&Options) -> Result<D>) {
    let mut connection: Option<Connection<D>> = None;
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
        let event = match execute(&mut connection, &mut connect, &options, command, &events).await {
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

async fn execute<D: Link>(
    connection: &mut Option<Connection<D>>,
    connect: &mut impl AsyncFnMut(&Options) -> Result<D>,
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
        None => {
            events.progress("Connecting to the keyboard...");
            let mut device = connect(options).await?;
            events.progress("Reading the keyboard info...");
            let info = device.info().await?;
            events.send(Event::Connected(info.clone()));
            ops::check_protocol(&info)?;
            connection.insert(Connection { device, info })
        }
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

/// Fake keyboard of the default size: answers like a real one, slowly; the written config outlives the connection
/// (`stored`), the factory one is the example config.
struct FakeDevice {
    factory: Rc<[u8]>,
    stored: Rc<RefCell<Vec<u8>>>,
    receiver: ConfigReceiver,
}

impl Link for FakeDevice {
    async fn request_once(&mut self, request: &Request) -> Result<Response> {
        sleep(Duration::from_millis(60)).await;
        Ok(match request {
            Request::GetInfo => Response::Info(Info {
                protocol_version: hid::PROTOCOL_VERSION,
                config_version: protocol::config::CONFIG_VERSION,
                width: ops::DEFAULT_SIZE.0,
                height: ops::DEFAULT_SIZE.1,
                max_layouts: MAX_LAYOUTS as u8,
                max_combos: MAX_COMBOS as u8,
                firmware_version: "0.0.0-fake".to_string(),
            }),
            Request::ReadConfig { offset } => Response::config_chunk(&self.stored.borrow(), *offset, hid::CHUNK_SIZE),
            Request::WriteBegin { len, crc32 } => self.receiver.begin(*len, *crc32).into(),
            Request::WriteChunk { offset, data } => self.receiver.chunk(*offset, data).into(),
            Request::WriteCommit => match self.receiver.commit() {
                Ok(config) => {
                    sleep(Duration::from_millis(500)).await;
                    *self.stored.borrow_mut() = config.to_bytes();
                    Response::Ok
                }
                Err(code) => Response::Error(code),
            },
            Request::ResetToDefault => {
                *self.stored.borrow_mut() = self.factory.to_vec();
                Response::Ok
            }
            Request::Ping | Request::End => Response::Ok,
        })
    }

    async fn close(self) {}
}
