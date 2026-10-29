//! ESP-IDF app image (`espflash save-image` output): what the firmware update needs from its header.
//!
//! Layout: the image header (`esp_image_header_t`, 24 bytes, chip ID at 12), the first segment header
//! (8 bytes), then the first segment starting with the app description (`esp_app_desc_t`): magic,
//! secure version, reserved, then `version` and `project_name` as NUL-terminated 32 bytes each.
//! With the hash appended flag (byte 23) the image ends with the SHA-256 of the bytes before it.
//!
//! The project name is the board the firmware is built for: a firmware installs only images of its own
//! board ([`AppInfo::is_for`]), reported in [`Versions::board`](crate::hid::Versions::board). The first board
//! is [`KEYBOARD_PROJECT`]; a firmware for another one (a new PCB revision, another chip) gets a project
//! name of its own (its crate name, or an app description of its own), so the old firmware refuses it.

use core::fmt;

/// First byte of an image.
pub const IMAGE_MAGIC: u8 = 0xE9;
/// First word of the app description.
pub const APP_DESC_MAGIC: u32 = 0xABCD_5432;

/// Chip ID of the ESP32-C3 in the image header.
pub const CHIP_ESP32C3: u16 = 0x0005;
/// Project name of the firmware of the first keyboard board (its crate name, set by `esp_app_desc!`).
pub const KEYBOARD_PROJECT: &str = "keyboard";

const CHIP_ID: usize = 12;
const HASH_APPENDED: usize = 23;
const APP_DESC: usize = 24 + 8;
const VERSION: usize = APP_DESC + 16;
const PROJECT_NAME: usize = VERSION + 32;

/// Bytes of the image start [`app_info`] reads.
pub const HEADER_LEN: usize = PROJECT_NAME + 32;

/// What an image is: chip and firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppInfo<'a> {
    pub chip_id: u16,
    pub project_name: &'a str,
    pub version: &'a str,
}

impl AppInfo<'_> {
    /// The image is a firmware of `board` (a project name, see the module docs).
    pub fn is_for(&self, board: &str) -> bool {
        self.chip_id == CHIP_ESP32C3 && self.project_name == board
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Shorter than [`HEADER_LEN`].
    Truncated,
    /// No image magic: not an app image (an ELF file, a merged flash image).
    Magic,
    /// No app description where it should be.
    AppDesc,
    /// Version or project name is not a NUL-terminated UTF-8 string.
    Text,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ImageError::Truncated => "image is truncated",
            ImageError::Magic => "not an ESP app image",
            ImageError::AppDesc => "image has no app description",
            ImageError::Text => "image has a broken app description",
        })
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ImageError {}

/// Reads the header of `image`, at least its first [`HEADER_LEN`] bytes.
pub fn app_info(image: &[u8]) -> Result<AppInfo<'_>, ImageError> {
    let header = image.get(..HEADER_LEN).ok_or(ImageError::Truncated)?;
    if header[0] != IMAGE_MAGIC {
        return Err(ImageError::Magic);
    }
    let app_magic = u32::from_le_bytes(header[APP_DESC..APP_DESC + 4].try_into().unwrap());
    if app_magic != APP_DESC_MAGIC {
        return Err(ImageError::AppDesc);
    }
    Ok(AppInfo {
        chip_id: u16::from_le_bytes([header[CHIP_ID], header[CHIP_ID + 1]]),
        project_name: c_str(&header[PROJECT_NAME..PROJECT_NAME + 32])?,
        version: c_str(&header[VERSION..VERSION + 32])?,
    })
}

/// Length of a [`build_id`].
pub const BUILD_ID_LEN: usize = 8;

/// Tells builds of one version apart: the first bytes of the SHA-256 the image ends with, the one the
/// firmware reads of its running image (`esp_partition_get_sha256`). `None` without the hash.
pub fn build_id(image: &[u8]) -> Option<[u8; BUILD_ID_LEN]> {
    if image.len() < HEADER_LEN + 32 || image[HASH_APPENDED] != 1 {
        return None;
    }
    image[image.len() - 32..][..BUILD_ID_LEN].try_into().ok()
}

fn c_str(field: &[u8]) -> Result<&str, ImageError> {
    let len = field.iter().position(|&b| b == 0).ok_or(ImageError::Text)?;
    core::str::from_utf8(&field[..len]).map_err(|_| ImageError::Text)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Image header of the keyboard firmware followed by `body`.
    pub fn sample(body: &[u8]) -> std::vec::Vec<u8> {
        let mut image = std::vec![0u8; HEADER_LEN];
        image[0] = IMAGE_MAGIC;
        image[CHIP_ID..CHIP_ID + 2].copy_from_slice(&CHIP_ESP32C3.to_le_bytes());
        image[APP_DESC..APP_DESC + 4].copy_from_slice(&APP_DESC_MAGIC.to_le_bytes());
        image[VERSION..VERSION + 5].copy_from_slice(b"0.1.0");
        image[PROJECT_NAME..PROJECT_NAME + KEYBOARD_PROJECT.len()].copy_from_slice(KEYBOARD_PROJECT.as_bytes());
        image.extend_from_slice(body);
        image
    }

    #[test]
    fn reads_header() {
        let image = sample(&[1, 2, 3]);
        let info = app_info(&image).unwrap();
        assert_eq!(info, AppInfo { chip_id: CHIP_ESP32C3, project_name: "keyboard", version: "0.1.0" });
        assert!(info.is_for(KEYBOARD_PROJECT));
        assert!(!info.is_for("keyboard-2"));
    }

    #[test]
    fn build_id_is_the_appended_hash() {
        let mut image = sample(&[0xAA; 40]);
        assert_eq!(build_id(&image), None);
        image[HASH_APPENDED] = 1;
        assert_eq!(build_id(&image), Some([0xAA; BUILD_ID_LEN]));
        let len = image.len();
        image[len - 32] = 1;
        assert_eq!(build_id(&image).unwrap()[0], 1);
        assert_eq!(build_id(&image[..HEADER_LEN]), None);
    }

    #[test]
    fn rejects_other_files() {
        let image = sample(&[]);
        assert_eq!(app_info(&image[..HEADER_LEN - 1]), Err(ImageError::Truncated));

        let mut elf = image.clone();
        elf[..4].copy_from_slice(b"\x7fELF");
        assert_eq!(app_info(&elf), Err(ImageError::Magic));

        let mut no_desc = image.clone();
        no_desc[APP_DESC] = 0;
        assert_eq!(app_info(&no_desc), Err(ImageError::AppDesc));

        let mut long_name = image.clone();
        long_name[PROJECT_NAME..PROJECT_NAME + 32].fill(b'a');
        assert_eq!(app_info(&long_name), Err(ImageError::Text));

        let mut other = image.clone();
        other[PROJECT_NAME..PROJECT_NAME + 9].copy_from_slice(b"joystick\0");
        assert!(!app_info(&other).unwrap().is_for(KEYBOARD_PROJECT));
    }
}
