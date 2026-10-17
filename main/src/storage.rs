use std::ffi::CString;
use esp_idf_sys::esp;

pub fn mount_vfs(path: &'static str, partition: &'static str) -> anyhow::Result<()> {
    let mount_config = esp_idf_sys::esp_vfs_fat_mount_config_t {
        max_files: 4,
        allocation_unit_size: esp_idf_sys::CONFIG_WL_SECTOR_SIZE as usize,
        disk_status_check_enable: false,
        format_if_mount_failed: true,
        use_one_fat: false
    };

    let mut handle = esp_idf_sys::WL_INVALID_HANDLE;
    let path = CString::new(path)?;
    let label = CString::new(partition)?;

    unsafe { esp!(esp_idf_sys::esp_vfs_fat_spiflash_mount(
        path.as_ptr(),
        label.as_ptr(),
        &mount_config as *const _,
        &mut handle as *mut _))?
    };

    Ok(())
}