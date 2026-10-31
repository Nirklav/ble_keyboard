//! The `storage` FAT partition and the files on it.

use std::ffi::CString;
use esp_idf_sys::esp;

/// Where the partition is mounted, the files below are in it.
macro_rules! root {
    () => {
        "/spiflash"
    };
}
const PARTITION: &str = "storage";

/// Saved config, see `config_store`.
pub const CONFIG_FILE: &str = concat!(root!(), "/config.bin");
/// Config being saved, see `config_store`.
pub const CONFIG_TMP: &str = concat!(root!(), "/config.tmp");
/// IRK sent to the hosts when they pair. The keyboard doesn't use resolvable addresses (see
/// [`crate::bt::profiles`]), so it only has to stay the same.
pub const IRK_FILE: &str = concat!(root!(), "/irk_keym.txt");
/// Last GATT database version the bonded hosts were told about, see `bt::gatt_cache`.
pub const GATT_DB_FILE: &str = concat!(root!(), "/gatt_db.txt");

/// Mounts the partition at `/spiflash`, formats it if it can't be mounted.
pub fn mount() -> anyhow::Result<()> {
    let mount_config = esp_idf_sys::esp_vfs_fat_mount_config_t {
        max_files: 4,
        allocation_unit_size: esp_idf_sys::CONFIG_WL_SECTOR_SIZE as usize,
        disk_status_check_enable: false,
        format_if_mount_failed: true,
        use_one_fat: false
    };

    let mut handle = esp_idf_sys::WL_INVALID_HANDLE;
    let path = CString::new(root!())?;
    let label = CString::new(PARTITION)?;

    unsafe { esp!(esp_idf_sys::esp_vfs_fat_spiflash_mount(
        path.as_ptr(),
        label.as_ptr(),
        &mount_config as *const _,
        &mut handle as *mut _))?
    };

    Ok(())
}