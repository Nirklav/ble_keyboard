//! Firmware update over the config protocol ([`protocol::firmware`]) into the other OTA slot.
//!
//! The requests are handled on a thread of their own: decompression and flash writes (a sector erase
//! each 4 KiB) don't belong in the NimBLE task, and [`esp_ota_end`](sys::esp_ota_end) reads the whole
//! image back to check it.
//!
//! On its first boot a new firmware is pending verification: it confirms itself on the first config
//! request ([`FirmwareUpdate::link_works`]), which shows that BLE, the HID reports and the protocol work.
//! Without one for [`ROLLBACK_TIMEOUT`] it goes back to the old firmware; a reset before that (a panic,
//! a failed init) makes the bootloader start the old one too. The keyboard doesn't deep sleep meanwhile
//! ([`pending_verification`]): a wake-up would be a reset.

use std::ffi::CStr;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_sys::{self as sys, esp, EspError};
use log::{error, info, warn};
use protocol::firmware::{FirmwareReceiver, ROLLBACK_TIMEOUT};
use protocol::hid::{self, ErrorCode, Request, Response, UpdateFeatures, Versions};
use protocol::image::BUILD_ID_LEN;

/// Time for the answer to [`Request::FirmwareCommit`] to reach the host before the restart.
const RESTART_DELAY: Duration = Duration::from_millis(500);
/// The decompressor state and the block are on the heap.
const STACK_SIZE: usize = 8 * 1024;
/// That of the main task: with the chunks of an update queued (see [`hid::CHUNK_WINDOW`]) the thread is busy for
/// long, the keys are scanned and the progress shown meanwhile. The default of the threads (5) starved the main loop.
const PRIORITY: u8 = 1;
/// Stack of the thread that starts a transfer: `InflateState::new_boxed` builds the decompressor state on the
/// stack before it moves to the heap, a frame of about 10 KiB in a release build and 43 KiB in a debug one.
/// Too much for [`STACK_SIZE`], and only needed for that moment.
const BEGIN_STACK_SIZE: usize = if cfg!(debug_assertions) { 48 * 1024 } else { 16 * 1024 };

/// The running firmware is new and not confirmed yet.
static PENDING: AtomicBool = AtomicBool::new(false);

/// Percent of the update received, [`NO_UPDATE`] without one; see [`update_progress`].
static PROGRESS: AtomicU8 = AtomicU8::new(NO_UPDATE);
const NO_UPDATE: u8 = u8::MAX;

enum Command {
    Request(Request),
    /// A config request came: the firmware works.
    Confirm,
    Abort,
}

/// The running firmware is pending verification, see the module docs. Known once [`FirmwareUpdate::spawn`]
/// returned, before that see [`running_state_pending`].
pub fn pending_verification() -> bool {
    PENDING.load(Ordering::Acquire)
}

/// Percent of the compressed image received, while an update is in progress; 100 from the last chunk until the
/// restart into the new firmware.
pub fn update_progress() -> Option<u8> {
    Some(PROGRESS.load(Ordering::Relaxed)).filter(|&progress| progress != NO_UPDATE)
}

fn set_progress(progress: Option<u8>) {
    PROGRESS.store(progress.unwrap_or(NO_UPDATE), Ordering::Relaxed);
}

/// Answer to [`Request::GetVersions`]: reads the whole running image for its hash, so it's made on the
/// first request, on the update thread.
fn versions() -> Versions {
    let slot = unsafe { sys::esp_ota_get_next_update_partition(ptr::null()).as_ref() };
    Versions {
        firmware_version: env!("CARGO_PKG_VERSION").into(),
        protocol_version: hid::PROTOCOL_VERSION,
        update_version: hid::UPDATE_VERSION,
        build_id: running_build_id(),
        board: running_board().into(),
        device_id: device_id(),
        slot_size: slot.map_or(0, |slot| slot.size),
        features: UpdateFeatures::empty().bits(),
    }
}

/// Board of the running firmware: the project name in its app description, see [`protocol::image`].
fn running_board() -> &'static str {
    // The app description is a static of the image.
    let desc = unsafe { &*sys::esp_app_get_description() };
    let name = unsafe { CStr::from_ptr(desc.project_name.as_ptr()) };
    name.to_str().unwrap_or("")
}

/// Bluetooth MAC address, zeros if it can't be read.
fn device_id() -> [u8; 6] {
    let mut mac = [0u8; 6];
    if let Err(e) = esp!(unsafe { sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_BT) }) {
        error!("Failed to read the Bluetooth MAC address: {}", e);
        return [0; 6];
    }
    mac
}

/// Handle of the update thread.
#[derive(Clone)]
pub struct FirmwareUpdate {
    commands: Sender<Command>,
}

impl FirmwareUpdate {
    /// Starts the thread; `respond` sends the answers to the requests, `on_update` is called when an update
    /// starts or ends ([`update_progress`] turns `Some` or `None`).
    pub fn spawn(
        respond: impl Fn(&Response) + Send + 'static,
        on_update: impl Fn() + Send + 'static,
    ) -> anyhow::Result<Self> {
        // Before any request can reach `link_works`.
        let pending = log_running();
        PENDING.store(pending, Ordering::Release);
        let (commands, receiver) = mpsc::channel();
        let default = ThreadSpawnConfiguration::get().unwrap_or_default();
        let own = ThreadSpawnConfiguration { priority: PRIORITY, ..ThreadSpawnConfiguration::get().unwrap_or_default() };
        own.set()?;
        let spawned = thread::Builder::new()
            .name("firmware".into())
            .stack_size(STACK_SIZE)
            .spawn(move || run(receiver, pending, respond, on_update));
        default.set()?;
        spawned?;
        Ok(Self { commands })
    }

    /// Handles a firmware request or [`Request::GetVersions`]; `respond` answers it.
    pub fn request(&self, request: Request) {
        // The thread lives as long as the program.
        let _ = self.commands.send(Command::Request(request));
    }

    /// A config request came, called before it's handled: confirms a firmware pending verification
    /// (before a [`Request::FirmwareBegin`] reaches the thread, the commands are in order).
    pub fn link_works(&self) {
        if pending_verification() {
            let _ = self.commands.send(Command::Confirm);
        }
    }

    /// Drops an unfinished update (e.g. on disconnect).
    pub fn abort(&self) {
        let _ = self.commands.send(Command::Abort);
    }
}

fn run(commands: Receiver<Command>, pending: bool, respond: impl Fn(&Response), on_update: impl Fn()) {
    let mut rollback_at = pending.then(|| Instant::now() + ROLLBACK_TIMEOUT);
    let mut versions = None;
    let mut update = Update::new();
    let mut updating = false;
    loop {
        // After the previous command, whichever way it went.
        if update_progress().is_some() != updating {
            updating = !updating;
            on_update();
        }
        let command = match rollback_at {
            Some(at) => match commands.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => {
                    rollback();
                    rollback_at = None;
                    continue;
                },
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match commands.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };

        let request = match command {
            Command::Request(request) => request,
            Command::Confirm => {
                // A failed confirmation is tried again on the next request, the rollback still comes without one.
                if rollback_at.is_some() && confirm() {
                    rollback_at = None;
                }
                continue;
            },
            Command::Abort => {
                if update.abort() {
                    info!("Firmware update aborted");
                }
                continue;
            },
        };
        if request == Request::GetVersions {
            respond(&Response::Versions(versions.get_or_insert_with(self::versions).clone()));
            continue;
        }
        let commit = request == Request::FirmwareCommit;
        let result = match request {
            Request::FirmwareBegin { len, crc32, image_len } => update.begin(len, crc32, image_len),
            Request::FirmwareChunk { offset, data } => update.chunk(offset, &data),
            Request::FirmwareCommit => update.commit(),
            request => {
                warn!("Firmware update: unexpected request {:?}", request);
                Err(ErrorCode::BadRequest)
            },
        };

        if let Err(code) = result {
            warn!("Firmware request failed: {}", code);
        }
        respond(&Response::from(result));
        if commit && result.is_ok() {
            info!("Firmware updated, restarting");
            thread::sleep(RESTART_DELAY);
            unsafe { sys::esp_restart() };
        }
    }
}

/// [`protocol::image::build_id`] of the running image, zeros if it can't be read.
fn running_build_id() -> [u8; BUILD_ID_LEN] {
    let mut hash = [0u8; 32];
    // Verifies the image and gives the hash it ends with: reads the whole image.
    match esp!(unsafe { sys::esp_partition_get_sha256(sys::esp_ota_get_running_partition(), hash.as_mut_ptr()) }) {
        Ok(()) => hash[..BUILD_ID_LEN].try_into().unwrap(),
        Err(e) => {
            error!("Failed to read the hash of the running firmware: {}", e);
            [0; BUILD_ID_LEN]
        }
    }
}

/// The running firmware is new and not confirmed yet, from its OTA state: before the update thread runs
/// too (unlike [`pending_verification`]), but stays true after the confirmation until the restart.
pub fn running_state_pending() -> bool {
    let mut state = sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED;
    unsafe {
        sys::esp_ota_get_state_partition(sys::esp_ota_get_running_partition(), &mut state) == sys::ESP_OK
            && state == sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
    }
}

/// Logs the running slot, returns whether it's pending verification.
fn log_running() -> bool {
    let pending = running_state_pending();
    let label = unsafe { CStr::from_ptr((*sys::esp_ota_get_running_partition()).label.as_ptr()) };
    info!("Running firmware from {}{}", label.to_string_lossy(), if pending { ", pending verification" } else { "" });
    pending
}

/// Returns whether it worked: the firmware stays pending verification otherwise (no deep sleep, see
/// [`pending_verification`]), a wake-up would make the bootloader go back to the old one.
fn confirm() -> bool {
    match esp!(unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() }) {
        Ok(()) => {
            info!("Firmware confirmed");
            PENDING.store(false, Ordering::Release);
            true
        },
        Err(e) => {
            error!("Failed to confirm the firmware: {}", e);
            false
        },
    }
}

/// Marks the running firmware invalid and restarts into the old one; keeps this one if there's nothing
/// to go back to.
fn rollback() {
    warn!("No config request since the firmware update, going back to the old firmware");
    // Returns only if it failed.
    let e = unsafe { sys::esp_ota_mark_app_invalid_rollback_and_reboot() };
    error!("Failed to go back to the old firmware ({}), keeping this one", e);
    confirm();
}

/// Update in progress: [`FirmwareReceiver`] and the slot it writes to, both set or both not.
struct Update {
    receiver: FirmwareReceiver,
    ota: Option<Ota>,
    /// Compressed length, for [`update_progress`].
    len: u32,
    stats: Stats,
}

/// Where the time of an update goes, logged on commit: the rest of it the thread waits for chunks.
struct Stats {
    started: Instant,
    /// Handling the chunks: decompressing and writing.
    chunks: Duration,
    /// Writing to the flash (a sector erase each block).
    writes: Duration,
}

impl Stats {
    fn new() -> Self {
        Self { started: Instant::now(), chunks: Duration::ZERO, writes: Duration::ZERO }
    }
}

impl Update {
    /// Takes images of the running firmware's board.
    fn new() -> Self {
        Self { receiver: FirmwareReceiver::new(running_board()), ota: None, len: 0, stats: Stats::new() }
    }

    /// Returns whether an update was in progress.
    fn abort(&mut self) -> bool {
        set_progress(None);
        self.receiver.abort();
        self.ota.take().is_some()
    }

    fn begin(&mut self, len: u32, crc32: u32, image_len: u32) -> Result<(), ErrorCode> {
        self.abort();
        let partition = unsafe { sys::esp_ota_get_next_update_partition(ptr::null()) };
        let Some(slot) = (unsafe { partition.as_ref() }) else {
            error!("No OTA slot to update");
            return Err(ErrorCode::Storage);
        };
        let max_image_len = slot.size as usize;
        let receiver = &mut self.receiver;
        thread::scope(|scope| {
            let begin = move || receiver.begin(len, crc32, image_len, max_image_len);
            match thread::Builder::new().stack_size(BEGIN_STACK_SIZE).spawn_scoped(scope, begin) {
                Ok(thread) => thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                Err(e) => {
                    error!("Can't start the firmware update thread: {}", e);
                    Err(ErrorCode::Busy)
                },
            }
        })?;
        match Ota::begin(partition) {
            Ok(ota) => {
                self.ota = Some(ota);
                self.len = len;
                self.stats = Stats::new();
                set_progress(Some(0));
            },
            Err(e) => {
                self.receiver.abort();
                error!("Failed to start the firmware update: {}", e);
                return Err(ErrorCode::Storage);
            },
        }
        let label = unsafe { CStr::from_ptr(slot.label.as_ptr()) }.to_string_lossy();
        info!(
            "Firmware update started: {} bytes ({} compressed) to {}, free heap {}",
            image_len,
            len,
            label,
            unsafe { sys::esp_get_free_heap_size() }
        );
        Ok(())
    }

    fn chunk(&mut self, offset: u32, data: &[u8]) -> Result<(), ErrorCode> {
        let ota = self.ota.as_mut().ok_or(ErrorCode::OutOfOrder)?;
        let start = Instant::now();
        let writes = &mut self.stats.writes;
        let result = self.receiver.chunk(offset, data, |block| {
            let start = Instant::now();
            let written = ota.write(block);
            *writes += start.elapsed();
            written
        });
        self.stats.chunks += start.elapsed();
        match result {
            Ok(()) => {
                let received = offset as u64 + data.len() as u64;
                set_progress(Some((received * 100 / self.len.max(1) as u64).min(100) as u8));
            },
            Err(_) => {
                self.ota = None;
                set_progress(None);
            },
        }
        result
    }

    fn commit(&mut self) -> Result<(), ErrorCode> {
        let mut ota = self.ota.take().ok_or(ErrorCode::OutOfOrder)?;
        let stats = &self.stats;
        let decompressing = stats.chunks.saturating_sub(stats.writes);
        info!(
            "Firmware received in {:.1?}: chunks took {:.1?} ({:.1?} decompressing, {:.1?} writing)",
            stats.started.elapsed(),
            stats.chunks,
            decompressing,
            stats.writes
        );
        let result = self.receiver.commit(|block| ota.write(block)).and_then(|()| ota.finish());
        if result.is_err() {
            set_progress(None);
        }
        result
    }
}

/// The slot is erased in blocks the flash erases at once, as the writes reach them: a sector erase per write
/// (`OTA_WITH_SEQUENTIAL_WRITES`) takes three times as long, and every erase stalls the NimBLE host (its code is
/// in the flash) while the chunks come. Not all at once on begin: that takes seconds for a large image, the app
/// waits for the answer meanwhile.
const ERASE_BLOCK: usize = 64 * 1024;

/// ESP-IDF OTA write, aborted on drop.
struct Ota {
    handle: sys::esp_ota_handle_t,
    partition: *const sys::esp_partition_t,
    written: usize,
    /// The slot is erased up to here.
    erased: usize,
}

impl Ota {
    fn begin(partition: *const sys::esp_partition_t) -> Result<Self, EspError> {
        let mut handle = 0;
        // A size, not `OTA_WITH_SEQUENTIAL_WRITES`: the writes don't erase then. Erases the first block.
        esp!(unsafe { sys::esp_ota_begin(partition, ERASE_BLOCK, &mut handle) })?;
        Ok(Self { handle, partition, written: 0, erased: ERASE_BLOCK })
    }

    fn write(&mut self, data: &[u8]) -> Result<(), ErrorCode> {
        let end = self.written + data.len();
        // The slot size is a multiple of the block; a write past it fails in `esp_ota_write`.
        let slot_size = unsafe { (*self.partition).size } as usize;
        while self.erased < end.min(slot_size) {
            esp!(unsafe { sys::esp_partition_erase_range(self.partition, self.erased, ERASE_BLOCK) }).map_err(|e| {
                error!("Firmware slot erase failed: {}", e);
                ErrorCode::Storage
            })?;
            self.erased += ERASE_BLOCK;
        }
        esp!(unsafe { sys::esp_ota_write(self.handle, data.as_ptr().cast(), data.len() as _) }).map_err(|e| {
            error!("Firmware write failed: {}", e);
            ErrorCode::Storage
        })?;
        self.written = end;
        Ok(())
    }

    /// Checks the written image and makes it the boot one.
    fn finish(self) -> Result<(), ErrorCode> {
        // `esp_ota_end` frees the handle whatever the result is.
        let this = ManuallyDrop::new(self);
        if let Err(e) = esp!(unsafe { sys::esp_ota_end(this.handle) }) {
            error!("Firmware image check failed: {}", e);
            return Err(if e.code() == sys::ESP_ERR_OTA_VALIDATE_FAILED as i32 {
                ErrorCode::InvalidImage
            } else {
                ErrorCode::Storage
            });
        }
        esp!(unsafe { sys::esp_ota_set_boot_partition(this.partition) }).map_err(|e| {
            error!("Failed to set the boot slot: {}", e);
            ErrorCode::Storage
        })
    }
}

impl Drop for Ota {
    fn drop(&mut self) {
        unsafe { sys::esp_ota_abort(self.handle) };
    }
}
