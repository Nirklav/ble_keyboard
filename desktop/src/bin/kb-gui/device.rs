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

use anyhow::{Context, Result};
use desktop::link::{Link, Options};
use desktop::ops::{self, FirmwareProgress, WriteProgress};
use desktop::time::{sleep, timeout};
use desktop::{Device, EXAMPLE_CONFIG, config_file};
use eframe::egui;
use futures::StreamExt;
use futures::channel::mpsc as async_mpsc;
use protocol::config::{Config, MAX_COMBOS, MAX_LAYOUTS};
use protocol::hid::{self, ConfigReceiver, ErrorCode, Info, Request, Response, Versions};

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
    /// Installs the bundled firmware, reconnects after the restart.
    UpdateFirmware,
    Disconnect,
}

/// Message from the worker.
#[derive(Debug)]
pub enum Event {
    /// Status of the running command.
    Progress(Busy),
    /// Connected to the keyboard (sent before the command continues).
    Connected(Versions, Info),
    /// The keyboard firmware is one the app can't edit, only update (sent before the command fails).
    Incompatible(Versions),
    /// [`Command::Connect`], [`Command::Reset`] or [`Command::UpdateFirmware`] is done, with a status text.
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
        !matches!(self, Event::Progress(_) | Event::Connected(..) | Event::Incompatible(_))
    }
}

/// Status of the running command: a text, and how much of it is done if that's known.
#[derive(Debug, Clone, PartialEq)]
pub struct Busy {
    pub text: String,
    /// 0..=1, `None` for a step of unknown length.
    pub fraction: Option<f32>,
}

impl Busy {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), fraction: None }
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
        self.send(Event::Progress(Busy::new(text)));
    }

    /// Progress of a step that is `done` of `total` through.
    fn progress_of(&self, text: impl Into<String>, done: usize, total: usize) {
        let fraction = if total == 0 { 1.0 } else { done as f32 / total as f32 };
        self.send(Event::Progress(Busy { text: text.into(), fraction: Some(fraction) }));
    }
}

/// Connection to a keyboard the app can edit, see [`open`].
struct Connection<D> {
    device: D,
    versions: Versions,
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
    match command {
        Command::Disconnect => {
            if let Some(old) = connection.take() {
                old.device.close().await;
            }
            return Ok(Event::Disconnected);
        }
        // Also for a keyboard the app can't edit, it doesn't need a connection.
        Command::UpdateFirmware => return update_firmware(connection, connect, options, events).await,
        _ => {}
    }
    let Connection { device, info, .. } = match connection {
        Some(connection) => connection,
        None => {
            events.progress("Connecting to the keyboard...");
            let mut device = connect(options).await?;
            events.progress("Reading the keyboard info...");
            let versions = device.versions().await?;
            connection.insert(open(device, versions, events).await?)
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
                WriteProgress::Sent { done, total } => {
                    events.progress_of(format!("Sent {done} of {total} bytes..."), done, total)
                }
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
        Command::Disconnect | Command::UpdateFirmware => unreachable!("handled above"),
    })
}

/// Connection to a keyboard the app can edit: speaks the app protocol, has a config version it reads
/// (also for Connect, which reads nothing: such a config is of a firmware the app doesn't know).
/// Otherwise closes `device`, sends [`Event::Incompatible`] and fails with the reason.
async fn open<D: Link>(mut device: D, versions: Versions, events: &EventSender) -> Result<Connection<D>> {
    let reason = match ops::check_protocol(versions.protocol_version) {
        Ok(()) => {
            let info = device.info().await?;
            match ops::check_config_version(&info) {
                Ok(()) => {
                    events.send(Event::Connected(versions.clone(), info.clone()));
                    return Ok(Connection { device, versions, info });
                }
                Err(reason) => reason,
            }
        }
        Err(reason) => reason,
    };
    // Nothing to do on it but the update, which connects itself: the keyboard may sleep meanwhile.
    device.close().await;
    events.send(Event::Incompatible(versions));
    Err(reason)
}

/// [`Command::UpdateFirmware`]: on the open connection or a new one; the keyboard restarts, the new connection
/// replaces the old one.
async fn update_firmware<D: Link>(
    connection: &mut Option<Connection<D>>,
    connect: &mut impl AsyncFnMut(&Options) -> Result<D>,
    options: &Options,
    events: &EventSender,
) -> Result<Event> {
    let firmware = ops::bundled_firmware().context("the app has no bundled firmware")?;
    let version = &firmware.version;
    // The session is gone with the restart.
    let (mut device, versions) = match connection.take() {
        Some(Connection { device, versions, .. }) => (device, versions),
        None => {
            events.progress("Connecting to the keyboard...");
            let mut device = connect(options).await?;
            let versions = device.versions().await?;
            (device, versions)
        }
    };
    events.progress(format!("Sending firmware {version}..."));
    // One event per percent, not per chunk (thousands of them).
    let mut last_percent = None;
    let sent = ops::update_firmware(&mut device, &versions, firmware, |progress| match progress {
        FirmwareProgress::Sent { done, total } => {
            let percent = done * 100 / total;
            if last_percent != Some(percent) {
                last_percent = Some(percent);
                events.progress_of(format!("Sending firmware {version}..."), done, total);
            }
        }
        FirmwareProgress::Checking => events.progress("Checking the firmware..."),
    })
    .await;
    if let Err(error) = sent {
        // The keyboard didn't restart: end the session, so it may sleep again.
        device.close().await;
        return Err(error);
    }
    drop(device);

    events.progress("Waiting for the keyboard to restart...");
    let (device, versions) = ops::reconnect_after_update(versions.device_id, async || connect(options).await).await?;
    if let Err(error) = firmware.check_running(&versions) {
        device.close().await;
        return Err(error);
    }
    *connection = Some(open(device, versions, events).await?);
    Ok(Event::Done(format!("Firmware {version} is running")))
}

const FAKE_FIRMWARE: &str = "0.0.0-fake";

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
                firmware_version: FAKE_FIRMWARE.to_string(),
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
            Request::GetVersions => Response::Versions(Versions {
                firmware_version: FAKE_FIRMWARE.to_string(),
                protocol_version: hid::PROTOCOL_VERSION,
                update_version: hid::UPDATE_VERSION,
                build_id: [0; protocol::image::BUILD_ID_LEN],
                board: protocol::image::KEYBOARD_PROJECT.to_string(),
                device_id: [0x02, 0, 0, 0, 0, 0x01],
                slot_size: 0x1E_0000,
                features: 0,
            }),
            Request::Ping | Request::End => Response::Ok,
            Request::FirmwareBegin { .. } | Request::FirmwareChunk { .. } | Request::FirmwareCommit => {
                Response::Error(ErrorCode::BadRequest)
            }
        })
    }

    async fn close(self) {}
}
