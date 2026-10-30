//! Connection to the keyboard config protocol ([`protocol::hid`]): hidapi natively ([`crate::transport`]),
//! WebHID in the browser ([`crate::webhid`]).

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use protocol::hid::{self, ErrorCode, Info, Request, Response, Versions};

use crate::time::sleep;

/// How often a [`ErrorCode::Busy`] request is retried.
const BUSY_RETRIES: usize = 5;
const BUSY_DELAY: Duration = Duration::from_millis(500);

/// Error of a request that didn't reach the keyboard: the write of its report failed.
#[derive(Debug, Clone, Copy)]
pub struct NotSent(pub &'static str);

impl std::fmt::Display for NotSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "can't send {}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Timeout of every request.
    pub timeout: Duration,
}

/// Connected keyboard. Futures are not `Send`: the links run on single-threaded executors.
#[allow(async_fn_in_trait)]
pub trait Link {
    /// Drops the responses that came already: late ones of timed out requests.
    fn drain(&mut self) -> Result<()>;

    /// Sends `request` without waiting for its response; the responses come in the order of the requests.
    async fn send(&mut self, request: &Request) -> Result<()>;

    /// Waits for the next response, to the request named `name` (for messages).
    async fn receive(&mut self, name: &'static str) -> Result<Response>;

    /// Sends `request` and waits for its response, no retries.
    async fn request_once(&mut self, request: &Request) -> Result<Response> {
        self.drain()?;
        self.send(request).await?;
        self.receive(name(request)).await
    }

    /// Ends the session (the keyboard may sleep again); keeps the OS connection.
    async fn close(self)
    where
        Self: Sized;

    /// Firmware versions: the first request, a firmware of any protocol version answers it
    /// (see the update part in [`protocol::hid`]).
    async fn versions(&mut self) -> Result<Versions> {
        match self.request(&Request::GetVersions).await {
            Ok(Response::Versions(versions)) => Ok(versions),
            Ok(response) => bail!("unexpected response to GetVersions: {response:?}"),
            // Firmware 0.1.0 has no GetVersions and no update yet: update version 0.
            Err(error) if error.downcast_ref::<ErrorCode>() == Some(&ErrorCode::BadRequest) => {
                let info = self.info().await?;
                Ok(Versions {
                    firmware_version: info.firmware_version,
                    protocol_version: info.protocol_version,
                    update_version: 0,
                    build_id: [0; protocol::image::BUILD_ID_LEN],
                    board: protocol::image::KEYBOARD_PROJECT.to_string(),
                    device_id: [0; 6],
                    slot_size: 0,
                    features: 0,
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Firmware info, only for a firmware of the app protocol version.
    async fn info(&mut self) -> Result<Info> {
        match self.request(&Request::GetInfo).await? {
            Response::Info(info) => Ok(info),
            response => bail!("unexpected response to GetInfo: {response:?}"),
        }
    }

    /// Retries while the keyboard is busy.
    /// [`Response::Error`] is returned as an error.
    async fn request(&mut self, request: &Request) -> Result<Response> {
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
    async fn request_ok(&mut self, request: &Request) -> Result<()> {
        match self.request(request).await? {
            Response::Ok => Ok(()),
            response => bail!("unexpected response to {}: {response:?}", name(request)),
        }
    }
}

/// Response in the message of an input report. An error code of a newer firmware (they are appended) is an
/// error of its own: the rest of the response can't be decoded.
pub fn decode_response(message: &[u8]) -> Result<Response> {
    Response::from_bytes(message).or_else(|error| match message {
        // `Response::Error` and the code, both varints; the tag is part of the update part, it stays.
        [3, code, ..] => bail!("the keyboard failed the request with error {code}, unknown to the app: update the app"),
        _ => Err(error).context("can't decode response"),
    })
}

/// Output report (without the report ID) with `request`.
pub fn encode(request: &Request) -> Result<[u8; hid::REPORT_LEN]> {
    let bytes = request.to_bytes();
    hid::encode(&bytes).ok_or_else(|| anyhow!("{} is {} bytes, doesn't fit a report", name(request), bytes.len()))
}

/// Name of the request for messages.
pub fn name(request: &Request) -> &'static str {
    match request {
        Request::GetInfo => "GetInfo",
        Request::ReadConfig { .. } => "ReadConfig",
        Request::WriteBegin { .. } => "WriteBegin",
        Request::WriteChunk { .. } => "WriteChunk",
        Request::WriteCommit => "WriteCommit",
        Request::ResetToDefault => "ResetToDefault",
        Request::Ping => "Ping",
        Request::End => "End",
        Request::FirmwareBegin { .. } => "FirmwareBegin",
        Request::FirmwareChunk { .. } => "FirmwareChunk",
        Request::FirmwareCommit => "FirmwareCommit",
        Request::GetVersions => "GetVersions",
    }
}
