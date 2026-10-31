//! Config of a connected keyboard whose firmware speaks the app protocol ([`hid::PROTOCOL_VERSION`]):
//! a [`Session`] is checked once, when it's opened, its operations rely on it.

use anyhow::{Context, Result, anyhow, bail, ensure};
use protocol::config::{self, Config};
use protocol::crc::crc32;
use protocol::hid::{self, Info, Request, Response, Versions};

use crate::compat::{check_config, check_config_version, check_protocol};
use crate::config_file;
use crate::link::{Client, Transport};

/// Progress of [`Session::write_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteProgress {
    /// `done` of `total` bytes are sent.
    Sent { done: usize, total: usize },
    /// All bytes are sent, the keyboard is applying the config (waits until all keys are released).
    Applying,
}

/// Connected keyboard of the app protocol, with its versions and info.
pub struct Session<T> {
    client: Client<T>,
    versions: Versions,
    info: Info,
}

/// What [`Session::open`] found.
pub enum Opened<T> {
    Session(Session<T>),
    /// The firmware speaks another protocol: the app can only update it, see [`crate::firmware`]. The session
    /// is ended (the keyboard may sleep again), the update connects itself.
    Incompatible { versions: Versions, reason: anyhow::Error },
}

impl<T: Transport> Session<T> {
    /// Session of a just connected keyboard: the versions first, the info only if the firmware speaks the
    /// app protocol. Fails if it doesn't, after ending the session (see [`Opened::Incompatible`]).
    pub async fn connect(mut client: Client<T>) -> Result<Session<T>> {
        let versions = client.versions().await?;
        match Session::open(client, versions).await? {
            Opened::Session(session) => Ok(session),
            Opened::Incompatible { reason, .. } => Err(reason),
        }
    }

    /// Session of a keyboard with `versions` ([`Client::versions`]) if its firmware speaks the app protocol,
    /// reads the info then. Otherwise ends the session, see [`Opened::Incompatible`].
    pub async fn open(mut client: Client<T>, versions: Versions) -> Result<Opened<T>> {
        if let Err(reason) = check_protocol(versions.protocol_version) {
            client.close().await;
            return Ok(Opened::Incompatible { versions, reason });
        }
        let info = client.info().await?;
        Ok(Opened::Session(Session { client, versions, info }))
    }

    pub fn versions(&self) -> &Versions {
        &self.versions
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// The client and the versions, e.g. for a firmware update.
    pub fn into_parts(self) -> (Client<T>, Versions) {
        (self.client, self.versions)
    }

    /// Ends the session, see [`Client::close`].
    pub async fn close(self) {
        self.client.close().await;
    }

    /// Keeps the session alive, see [`hid::PING_INTERVAL`].
    pub async fn ping(&mut self) -> Result<()> {
        self.client.request_ok(&Request::Ping).await
    }

    /// Reads, decodes and validates the keyboard config, an older version is upgraded to the app one.
    pub async fn read_config(&mut self) -> Result<Config> {
        check_config_version(&self.info)?;
        let bytes = self.read_config_bytes().await?;
        let config = Config::from_bytes(&bytes).context("can't decode the keyboard config")?;
        config
            .validate_for(self.info.width, self.info.height)
            .map_err(|e| anyhow!("keyboard config is invalid: {}", config_file::describe(&e, &config)))?;
        Ok(config)
    }

    async fn read_config_bytes(&mut self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut expected_len = None;
        loop {
            let offset = bytes.len() as u32;
            let response = self.client.request(&Request::ReadConfig { offset }).await?;
            let Response::ConfigChunk { total_len, offset: chunk_offset, data } = response else {
                bail!("unexpected response to ReadConfig: {response:?}");
            };
            ensure!(chunk_offset == offset, "asked for offset {offset}, got {chunk_offset}");
            ensure!(total_len as usize <= hid::MAX_CONFIG_SIZE, "config of {total_len} bytes is too large");
            if *expected_len.get_or_insert(total_len) != total_len {
                bail!("config changed while reading, try again");
            }

            bytes.extend_from_slice(&data);
            ensure!(bytes.len() <= total_len as usize, "keyboard sent more than {total_len} bytes");
            if bytes.len() == total_len as usize {
                return Ok(bytes);
            }
            ensure!(!data.is_empty(), "keyboard sent an empty chunk at {offset} of {total_len}");
        }
    }

    /// Validates `config` against the keyboard first. Returns the number of bytes sent.
    pub async fn write_config(&mut self, config: &Config, mut progress: impl FnMut(WriteProgress)) -> Result<usize> {
        let info = &self.info;
        ensure!(
            info.config_version == config::CONFIG_VERSION,
            "keyboard config version {} differs from the app one {}, update the firmware or the app",
            info.config_version,
            config::CONFIG_VERSION
        );
        check_config(config, Some(info)).map_err(anyhow::Error::msg)?;

        let client = &mut self.client;
        let bytes = config.to_bytes();
        let total = bytes.len();
        client.request_ok(&Request::WriteBegin { len: total as u32, crc32: crc32(&bytes) }).await?;
        for (offset, chunk) in hid::chunks(&bytes, hid::CHUNK_SIZE) {
            client.request_ok(&Request::WriteChunk { offset, data: chunk.to_vec() }).await?;
            progress(WriteProgress::Sent { done: offset as usize + chunk.len(), total });
        }
        progress(WriteProgress::Applying);
        client.request_ok(&Request::WriteCommit).await?;
        Ok(total)
    }

    /// Restores the factory config on the keyboard.
    pub async fn reset_config(&mut self) -> Result<()> {
        self.client.request_ok(&Request::ResetToDefault).await
    }
}
