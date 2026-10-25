//! BLE connection to the keyboard config service (see [`protocol::ble`]).
//!
//! Device discovery: the keyboard is normally already connected to the computer as a HID
//! device and doesn't advertise while connected (it allows one connection), so scanning alone
//! won't find it. The app reuses the OS connection:
//! 1. [`Central::retrieve_peripherals`] with the config service UUID: on Windows this
//!    enumerates the connected BLE devices and checks their (cached) GATT services;
//! 2. otherwise a scan, matching the name of the discovered / known peripherals
//!    (also covers Linux, where BlueZ lists paired devices among the peripherals).
//!
//! The connection is never closed explicitly: `disconnect` would drop the HID link on Linux.

use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, RetrievePeripheralsOptions, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use protocol::ble::{self, ErrorCode, Info, Request, Response};
use tokio::time::{Instant, sleep, timeout};
use uuid::Uuid;

const SERVICE: Uuid = Uuid::from_u128(ble::SERVICE_UUID);
const INFO: Uuid = Uuid::from_u128(ble::INFO_UUID);
const CONTROL: Uuid = Uuid::from_u128(ble::CONTROL_UUID);
const RESPONSE: Uuid = Uuid::from_u128(ble::RESPONSE_UUID);

/// How often a [`ErrorCode::Busy`] request is retried.
const BUSY_RETRIES: usize = 5;
const BUSY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct Options {
    /// Device name filter (case-insensitive substring).
    pub name: String,
    /// Timeout of discovery and of every request.
    pub timeout: Duration,
    /// ATT MTU override; `None` uses the negotiated one.
    pub mtu: Option<usize>,
}

type Notifications = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;

/// Connected keyboard with the config service.
pub struct Device {
    peripheral: Peripheral,
    info: Characteristic,
    control: Characteristic,
    response: Characteristic,
    notifications: Notifications,
    timeout: Duration,
    mtu: usize,
}

impl Device {
    /// Finds the keyboard, connects and subscribes to responses.
    pub async fn connect(options: &Options) -> Result<Device> {
        let manager = Manager::new().await.context("Bluetooth is not available")?;
        let adapter =
            manager.adapters().await?.into_iter().next().ok_or_else(|| anyhow!("no Bluetooth adapter found"))?;

        let peripheral = find(&adapter, options).await?;
        let name = peripheral_name(&peripheral).await.unwrap_or_else(|| "keyboard".into());
        eprintln!("Connecting to {name} ({})...", peripheral.address());

        timeout(options.timeout, peripheral.connect())
            .await
            .context("connection timed out")?
            .context("can't connect")?;
        timeout(options.timeout, peripheral.discover_services())
            .await
            .context("service discovery timed out")?
            .context("service discovery failed")?;

        let characteristic = |uuid: Uuid| {
            peripheral.characteristics().into_iter().find(|c| c.service_uuid == SERVICE && c.uuid == uuid).ok_or_else(
                || {
                    anyhow!(
                        "{name} has no config service characteristic {uuid}: the firmware is too old, \
                         or the OS cached the old GATT services (remove and pair the keyboard again)"
                    )
                },
            )
        };
        let info = characteristic(INFO)?;
        let control = characteristic(CONTROL)?;
        let response = characteristic(RESPONSE)?;

        // Take the stream before subscribing, so that no notification is missed.
        let notifications = peripheral.notifications().await?;
        timeout(options.timeout, peripheral.subscribe(&response))
            .await
            .context("subscribe timed out")?
            .context("can't subscribe to responses")?;

        let mtu = options.mtu.unwrap_or_else(|| (peripheral.mtu() as usize).max(ble::DEFAULT_MTU));
        Ok(Device { peripheral, info, control, response, notifications, timeout: options.timeout, mtu })
    }

    /// Reads the info characteristic.
    pub async fn info(&self) -> Result<Info> {
        let bytes = timeout(self.timeout, self.peripheral.read(&self.info))
            .await
            .context("reading info timed out")?
            .context("can't read info")?;
        Info::from_bytes(&bytes).context("can't decode info, firmware protocol is not supported")
    }

    /// ATT MTU used for chunking.
    pub fn mtu(&self) -> usize {
        self.mtu
    }

    /// Max number of config bytes per chunk.
    pub fn chunk_size(&self) -> usize {
        ble::chunk_payload_size(self.mtu)
    }

    /// Retries while the keyboard is busy.
    /// [`Response::Error`] is returned as an error.
    pub async fn request(&mut self, request: &Request) -> Result<Response> {
        for attempt in 0.. {
            match self.request_once(request).await? {
                Response::Error(ErrorCode::Busy) if attempt < BUSY_RETRIES => sleep(BUSY_DELAY).await,
                Response::Error(code) => {
                    return Err(code).with_context(|| format!("{} failed", name(request)));
                }
                response => return Ok(response),
            }
        }
        unreachable!()
    }

    /// Sends `request` and expects [`Response::Ok`].
    pub async fn request_ok(&mut self, request: &Request) -> Result<()> {
        match self.request(request).await? {
            Response::Ok => Ok(()),
            response => bail!("unexpected response to {}: {response:?}", name(request)),
        }
    }

    async fn request_once(&mut self, request: &Request) -> Result<Response> {
        let bytes = request.to_bytes();
        if bytes.len() > self.mtu - ble::ATT_HEADER {
            bail!("{} is {} bytes, doesn't fit MTU {}", name(request), bytes.len(), self.mtu);
        }

        timeout(self.timeout, self.peripheral.write(&self.control, &bytes, WriteType::WithResponse))
            .await
            .with_context(|| format!("{} write timed out", name(request)))?
            .with_context(|| format!("can't send {}", name(request)))?;

        let deadline = Instant::now() + self.timeout;
        loop {
            let notification = tokio::time::timeout_at(deadline, self.notifications.next())
                .await
                .with_context(|| format!("no response to {} within {:?}", name(request), self.timeout))?
                .ok_or_else(|| anyhow!("keyboard disconnected"))?;
            if notification.uuid == RESPONSE {
                return Response::from_bytes(&notification.value).context("can't decode response");
            }
        }
    }

    /// Ends the session (the keyboard may sleep again) and unsubscribes from responses;
    /// keeps the OS connection.
    pub async fn close(mut self) {
        let _ = self.request_ok(&Request::End).await;
        let _ = timeout(self.timeout, self.peripheral.unsubscribe(&self.response)).await;
    }
}

fn name(request: &Request) -> &'static str {
    match request {
        Request::GetInfo => "GetInfo",
        Request::ReadConfig { .. } => "ReadConfig",
        Request::WriteBegin { .. } => "WriteBegin",
        Request::WriteChunk { .. } => "WriteChunk",
        Request::WriteCommit => "WriteCommit",
        Request::ResetToDefault => "ResetToDefault",
        Request::Ping => "Ping",
        Request::End => "End",
    }
}

async fn peripheral_name(peripheral: &Peripheral) -> Option<String> {
    let properties = peripheral.properties().await.ok().flatten()?;
    properties.local_name.or(properties.advertisement_name)
}

/// Name check: `Some(true/false)` if the name is known, `None` otherwise.
async fn name_matches(peripheral: &Peripheral, filter: &str) -> Option<bool> {
    let name = peripheral_name(peripheral).await?;
    Some(name.to_lowercase().contains(&filter.to_lowercase()))
}

/// Finds the keyboard: connected devices with the config service first, then a scan by name.
async fn find(adapter: &Adapter, options: &Options) -> Result<Peripheral> {
    let retrieve = RetrievePeripheralsOptions { identifiers: None, services: Some(vec![SERVICE]) };
    // Not supported by every backend; the scan below is the fallback.
    if let Ok(Ok(connected)) = timeout(options.timeout, adapter.retrieve_peripherals(retrieve)).await {
        // Connected devices usually have no known name (nothing was advertised), so only
        // the ones with a known different name are skipped.
        let mut candidates = Vec::new();
        for peripheral in connected {
            if name_matches(&peripheral, &options.name).await != Some(false) {
                candidates.push(peripheral);
            }
        }
        if candidates.len() > 1 {
            eprintln!("Several connected devices have the config service, using the first one");
        }
        if let Some(peripheral) = candidates.into_iter().next() {
            return Ok(peripheral);
        }
    }

    eprintln!("Looking for \"{}\"...", options.name);
    adapter.start_scan(ScanFilter::default()).await.context("can't start scanning")?;
    let found = scan_by_name(adapter, &options.name, options.timeout).await;
    let _ = adapter.stop_scan().await;
    found?.ok_or_else(|| {
        anyhow!("keyboard \"{}\" not found: check that it is paired and connected, or pass --name", options.name)
    })
}

async fn scan_by_name(adapter: &Adapter, filter: &str, limit: Duration) -> Result<Option<Peripheral>> {
    let deadline = Instant::now() + limit;
    loop {
        for peripheral in adapter.peripherals().await? {
            if name_matches(&peripheral, filter).await == Some(true) {
                return Ok(Some(peripheral));
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        sleep(Duration::from_millis(250)).await;
    }
}
