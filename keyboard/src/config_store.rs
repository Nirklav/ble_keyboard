//! Config file on the FAT partition: `magic | format_version: u16 | len: u32 | crc32: u32`
//! (little-endian), then `len` bytes of postcard payload ([`Config::to_bytes`]).

use std::fmt;
use std::fs;
use std::io::{self, Write};

use protocol::config::{crc32, Config, ValidationError, CONFIG_VERSION};

const CONFIG_FILE: &str = "/spiflash/config.bin";
const CONFIG_TMP: &str = "/spiflash/config.tmp";

const MAGIC: [u8; 4] = *b"KBCF";
const FORMAT_VERSION: u16 = 1;
const HEADER_LEN: usize = 14;

pub fn encode(config: &Config) -> Vec<u8> {
    let payload = config.to_bytes();
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&crc32(&payload).to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

/// Decodes and validates a config for a `width` x `height` keyboard.
/// Also returns the version it was stored with, before [`Config::from_bytes`] upgraded it.
pub fn decode(bytes: &[u8], width: u8, height: u8) -> Result<(Config, u16), LoadError> {
    if bytes.len() < HEADER_LEN {
        return Err(LoadError::Truncated);
    }
    let (header, payload) = bytes.split_at(HEADER_LEN);

    if header[0..4] != MAGIC {
        return Err(LoadError::Magic);
    }

    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != FORMAT_VERSION {
        return Err(LoadError::FormatVersion(version));
    }

    let len = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as usize;
    if len != payload.len() {
        return Err(LoadError::Len { expected: len, found: payload.len() });
    }

    let crc = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
    if crc != crc32(payload) {
        return Err(LoadError::Crc);
    }

    let (stored_version, _) = postcard::take_from_bytes::<u16>(payload).map_err(LoadError::Decode)?;
    let config = Config::from_bytes(payload).map_err(LoadError::Decode)?;
    config.validate_for(width, height).map_err(LoadError::Invalid)?;
    Ok((config, stored_version))
}

/// Reads the saved config, `Ok(None)` if there is none.
/// A config stored with an older version is saved again upgraded if `save_upgraded`, otherwise it's
/// upgraded on every load until then.
pub fn load(width: u8, height: u8, save_upgraded: bool) -> Result<Option<Config>, LoadError> {
    let Some((config, stored_version)) = load_stored(width, height)? else {
        return Ok(None);
    };

    if stored_version != CONFIG_VERSION {
        if save_upgraded {
            log::info!("Saving config upgraded from version {} to {}", stored_version, CONFIG_VERSION);
            if let Err(e) = save(&config) {
                log::error!("Failed to save upgraded config: {}", e);
            }
        } else {
            log::info!("Config upgraded from version {} to {}, saved once the firmware is confirmed", stored_version, CONFIG_VERSION);
        }
    }
    Ok(Some(config))
}

/// Falls back to the temporary file left by an interrupted [`save`].
fn load_stored(width: u8, height: u8) -> Result<Option<(Config, u16)>, LoadError> {
    let saved = read(CONFIG_FILE, width, height);
    if let Ok(Some(_)) = saved {
        return saved;
    }

    match read(CONFIG_TMP, width, height) {
        Ok(Some(loaded)) => {
            log::warn!("Using {}, {} is missing or broken", CONFIG_TMP, CONFIG_FILE);
            if let Err(e) = commit_tmp() {
                log::error!("Failed to rename {}: {}", CONFIG_TMP, e);
            }
            Ok(Some(loaded))
        },
        Ok(None) => saved,
        Err(e) => {
            log::error!("{} is broken: {}", CONFIG_TMP, e);
            match saved {
                Ok(None) => Err(e),
                saved => saved
            }
        }
    }
}

/// `Ok(None)` if there is no `path`.
fn read(path: &str, width: u8, height: u8) -> Result<Option<(Config, u16)>, LoadError> {
    match fs::read(path) {
        Ok(bytes) => decode(&bytes, width, height).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into())
    }
}

/// Writes the temporary file, then replaces the config file with it.
/// FAT can't rename over an existing file: if the power is lost between removing the old
/// file and renaming, [`load`] picks up the complete temporary file.
pub fn save(config: &Config) -> io::Result<()> {
    let mut file = fs::File::create(CONFIG_TMP)?;
    file.write_all(&encode(config))?;
    file.sync_all()?;
    drop(file);
    commit_tmp()
}

fn commit_tmp() -> io::Result<()> {
    remove(CONFIG_FILE)?;
    fs::rename(CONFIG_TMP, CONFIG_FILE)
}

/// The factory config is used from now on.
pub fn remove_saved() -> io::Result<()> {
    remove(CONFIG_TMP)?;
    remove(CONFIG_FILE)
}

fn remove(path: &str) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(())
    }
}

#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    Truncated,
    Magic,
    FormatVersion(u16),
    Len { expected: usize, found: usize },
    Crc,
    Decode(postcard::Error),
    Invalid(ValidationError),
}

impl From<io::Error> for LoadError {
    fn from(e: io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "io: {}", e),
            LoadError::Truncated => write!(f, "file is shorter than the header"),
            LoadError::Magic => write!(f, "wrong magic"),
            LoadError::FormatVersion(v) => write!(f, "unsupported format version {}", v),
            LoadError::Len { expected, found } => write!(f, "payload is {} bytes, header says {}", found, expected),
            LoadError::Crc => write!(f, "crc mismatch"),
            LoadError::Decode(e) => write!(f, "decode: {}", e),
            LoadError::Invalid(e) => write!(f, "invalid: {}", e),
        }
    }
}
