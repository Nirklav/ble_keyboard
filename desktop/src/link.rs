//! Connection to the keyboard config protocol ([`protocol::hid`]): hidapi natively ([`crate::transport`]),
//! WebHID in the browser ([`crate::webhid`]).

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use protocol::hid::{self, ErrorCode, Info, Request, Response};

use crate::time::sleep;

/// How often a [`ErrorCode::Busy`] request is retried.
const BUSY_RETRIES: usize = 5;
const BUSY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct Options {
    /// Timeout of every request.
    pub timeout: Duration,
}

/// Connected keyboard. Futures are not `Send`: the links run on single-threaded executors.
#[allow(async_fn_in_trait)]
pub trait Link {
    /// Sends `request` and waits for its response, no retries.
    async fn request_once(&mut self, request: &Request) -> Result<Response>;

    /// Max number of config bytes per chunk.
    fn chunk_size(&self) -> usize;

    /// Ends the session (the keyboard may sleep again); keeps the OS connection.
    async fn close(self)
    where
        Self: Sized;

    /// Firmware info.
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
    }
}
