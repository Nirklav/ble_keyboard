//! BLE host profiles: the keyboard remembers up to [`PROFILES`] hosts and is connected to the one
//! of the active profile.
//!
//! Every profile has its own static random address, so to a host each profile is a separate device:
//! a host bonded to one profile never sees the address it knows while another profile is active and
//! doesn't grab the connection, and it caches the GATT services of its profile only. There's no privacy
//! (RPA): a resolvable address would be resolved with the one local IRK by every bonded host.
//!
//! A profile is bound to at most one host. An empty profile accepts any host that pairs and binds it;
//! a bound one rejects a different host that pairs (its new bond is removed).
//!
//! NimBLE keeps one bond per host, not per local address, and esp32-nimble deletes the bond of a host
//! that pairs again. So a host bound to one profile is dropped as soon as it connects to the address of
//! another one ([`Profiles::bound_elsewhere`]), and its pairing is refused until it's gone
//! ([`refuse_pairing`]): it keeps its bond and profile. To move a host, its profile is cleared first.
//! That needs the host identified on connect: by its identity address, or by its resolvable address,
//! which the controller resolves with the IRKs of the bonds. A host that changed its IRK isn't
//! recognized and pairs as a new one; NimBLE then replaces its bond, the profile it's bound to keeps it
//! (see [`Profiles::on_encrypted`]) and the host has to pair again there.
//!
//! Clearing a profile removes the bond and gives the profile a new address, the old host doesn't know
//! the new one and can't connect to it; it has to remove the keyboard and pair again.
//!
//! Only bonded hosts are bound: a peer that pairs without bonding is disconnected (see
//! [`Profiles::on_encrypted`]), so an empty profile stays empty and a bound one keeps its host.
//!
//! The profiles and the active one are saved in NVS. If NVS can't be read (not just empty), the keyboard
//! runs on new profiles kept in RAM only, and leaves the bonds alone: the saved profiles and the bonds
//! are intact for the next boot. Locked by the main loop, the NimBLE host task and the link thread
//! (see `ble_mouse_keyboard`).

use std::ffi::{c_int, c_void, CString};
use std::sync::{Mutex, OnceLock};

use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_sys::{
    ble_addr_t, ble_store_key, ble_store_value, esp, EspError, ESP_ERR_NVS_INVALID_LENGTH, ESP_ERR_NVS_TYPE_MISMATCH,
};
use log::{error, info, warn};

pub use protocol::config::BT_PROFILES as PROFILES;

const NAMESPACE: &str = "bt_profiles";
const KEY: &str = "profiles";
/// Format of the saved blob: `FORMAT | active | PROFILES x (address[6] | host type | host[6])`,
/// host type [`NO_HOST`] if the profile has no host.
const FORMAT: u8 = 1;
const NO_HOST: u8 = 0xFF;
const PROFILE_LEN: usize = 13;
const BLOB_LEN: usize = 2 + PROFILES * PROFILE_LEN;

/// Identity address of a host, as NimBLE keeps its bond.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Host {
    /// `BLE_ADDR_PUBLIC` or `BLE_ADDR_RANDOM`.
    addr_type: u8,
    /// Little-endian, like NimBLE.
    addr: [u8; 6],
}

impl Host {
    /// The `_ID` address types of a connection are the same identity.
    fn new(addr: &ble_addr_t) -> Self {
        Self { addr_type: addr.type_ & 1, addr: addr.val }
    }

    fn ble_addr(&self) -> ble_addr_t {
        ble_addr_t { type_: self.addr_type, val: self.addr }
    }
}

#[derive(Debug, Clone, Copy)]
struct Profile {
    /// Own static random address, little-endian.
    address: [u8; 6],
    host: Option<Host>,
}

impl Profile {
    fn new() -> Self {
        Self { address: random_static_address(), host: None }
    }

    /// Forgets the host (not its bond) and takes a new address: the old host can't find the profile.
    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Active profile, for the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileState {
    pub index: u8,
    /// Has a host, otherwise waits for a new one to pair.
    pub bound: bool,
}

/// What to do with a connection that just got encrypted, see [`Profiles::on_encrypted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    /// The connection is on the address of another profile than the active one (it's being switched),
    /// the peer hasn't bonded, or it's bound to another profile.
    Disconnect,
    /// A host bound to no profile has paired with a bound one: its bond is removed and it's disconnected.
    Reject,
}

/// What [`Profiles::load`] found in NVS.
enum Saved {
    Profiles(usize, [Profile; PROFILES]),
    /// First boot, or saved by a firmware with another format: new profiles replace them.
    Nothing,
    /// NVS failed: nothing can be told about the saved profiles.
    Unreadable,
}

pub struct Profiles {
    /// `None` if NVS couldn't be read: the profiles aren't saved then, not to overwrite the saved ones.
    nvs: Option<EspNvs<NvsDefault>>,
    /// Last blob read from or written to NVS, so unchanged profiles aren't written again.
    saved: Option<[u8; BLOB_LEN]>,
    active: usize,
    profiles: [Profile; PROFILES],
}

impl Profiles {
    /// Loads the saved profiles, or makes new ones. Must be called after NimBLE has started:
    /// removes the bonds of hosts no profile knows (e.g. made by older firmware), forgets hosts without bonds.
    /// If NVS fails, new profiles are kept in RAM only and the bonds are left alone (see the module docs).
    pub fn load(partition: EspDefaultNvsPartition) -> Self {
        let (nvs, saved) = match EspNvs::new(partition, NAMESPACE, true) {
            Ok(nvs) => {
                let saved = read(&nvs);
                (Some(nvs), saved)
            }
            Err(e) => {
                error!("BT profiles: can't open NVS: {}", e);
                (None, Saved::Unreadable)
            }
        };

        let new_profiles = || core::array::from_fn(|_| Profile::new());
        let mut profiles = match saved {
            Saved::Profiles(active, profiles) => {
                let mut profiles = Self { nvs, saved: None, active, profiles };
                profiles.saved = Some(profiles.encode());
                profiles
            }
            Saved::Nothing => {
                info!("BT profiles: none saved, new addresses");
                Self { nvs, saved: None, active: 0, profiles: new_profiles() }
            }
            Saved::Unreadable => {
                error!("BT profiles: running on temporary profiles, not saved; the bonds are kept");
                Self { nvs: None, saved: None, active: 0, profiles: new_profiles() }
            }
        };
        if profiles.nvs.is_some() {
            profiles.sync_bonds();
            profiles.save();
        }
        for (index, profile) in profiles.profiles.iter().enumerate() {
            info!("BT profile {}: address {}, host {:?}", index + 1, format(&profile.address), profile.host.map(|h| format(&h.addr)));
        }
        profiles
    }

    pub fn state(&self) -> ProfileState {
        ProfileState { index: self.active as u8, bound: self.profiles[self.active].host.is_some() }
    }

    /// Makes `index` the active profile, `None` if it's out of range or already active.
    /// The connection has to be dropped, then [`Profiles::apply_address`].
    pub fn select(&mut self, index: u8) -> Option<ProfileState> {
        let index = index as usize;
        if index >= PROFILES || index == self.active {
            return None;
        }
        self.active = index;
        self.save();
        info!("BT profile {} selected", index + 1);
        Some(self.state())
    }

    /// Next profile (`step` 1) or the previous one (`step` -1), wrapping around.
    pub fn step(&mut self, step: isize) -> Option<ProfileState> {
        let index = (self.active as isize + step).rem_euclid(PROFILES as isize);
        self.select(index as u8)
    }

    /// Forgets the host of the active profile and gives the profile a new address.
    /// The connection has to be dropped, then [`Profiles::apply_address`].
    pub fn clear_active(&mut self) -> ProfileState {
        self.release(self.active);
        self.save();
        info!("BT profile {} cleared", self.active + 1);
        self.state()
    }

    /// Forgets all hosts and every bond NimBLE has, every profile gets a new address.
    /// The connection has to be dropped, then [`Profiles::apply_address`].
    pub fn clear_all(&mut self) -> ProfileState {
        for index in 0..PROFILES {
            self.release(index);
        }
        let rc = unsafe { esp_idf_sys::ble_store_clear() };
        if rc != 0 {
            error!("BT profiles: ble_store_clear failed: {}", rc);
        }
        self.save();
        info!("BT profiles: all cleared");
        self.state()
    }

    /// Sets the address of the active profile in NimBLE and the controller, if it isn't set yet.
    /// Only while not advertising; while connected only if the connection is being dropped.
    /// On a failure the previous address stays, the next call tries again.
    pub fn apply_address(&mut self) {
        let address = self.profiles[self.active].address;
        if own_address() == Some(address) {
            return;
        }
        let rc = unsafe { esp_idf_sys::ble_hs_id_set_rnd(address.as_ptr()) };
        if rc == 0 {
            info!("BT profile {}: address {}", self.active + 1, format(&address));
        } else {
            error!("BT profiles: can't set address {}: {}", format(&address), rc);
        }
    }

    /// The connection with own address `local` belongs to the active profile.
    pub fn is_active_address(&self, local: &ble_addr_t) -> bool {
        local.val == self.profiles[self.active].address
    }

    /// `peer` (identity address), just connected, is bound to another profile than the active one: it has
    /// to be disconnected before it pairs, see the module docs.
    pub fn bound_elsewhere(&self, peer: &ble_addr_t) -> bool {
        let peer = Host::new(peer);
        match self.profile_of(peer) {
            Some(index) if index != self.active => {
                info!("BT profiles: {} is bound to profile {}, disconnecting", format(&peer.addr), index + 1);
                true
            }
            _ => false,
        }
    }

    /// `peer` (identity address) has encrypted the connection on own address `local`, maybe after pairing.
    /// Binds a bonded peer to an empty active profile, checks that a bound one is its own.
    pub fn on_encrypted(&mut self, peer: &ble_addr_t, local: &ble_addr_t, bonded: bool) -> Verdict {
        let peer = Host::new(peer);
        if !self.is_active_address(local) {
            info!("BT profiles: {} connected to a previous profile", format(&peer.addr));
            return Verdict::Disconnect;
        }
        if !bonded {
            // Nothing is stored for it, so there would be nothing to recognize it by.
            warn!("BT profiles: {} hasn't bonded, disconnecting", format(&peer.addr));
            return Verdict::Disconnect;
        }
        if let Some(index) = self.profile_of(peer).filter(|&index| index != self.active) {
            // Not recognized on connect (see the module docs), so it may have just paired here and NimBLE
            // replaced its bond. Its profile keeps it all the same: the host pairs there again if needed.
            warn!("BT profiles: {} is bound to profile {}, disconnecting", format(&peer.addr), index + 1);
            return Verdict::Disconnect;
        }

        match self.profiles[self.active].host {
            Some(host) if host == peer => Verdict::Accept,
            Some(host) => {
                warn!(
                    "BT profile {} is bound to {}, rejecting {}",
                    self.active + 1,
                    format(&host.addr),
                    format(&peer.addr)
                );
                // Bound to no profile (checked above), the bond is the one it has just made.
                let rc = unsafe { esp_idf_sys::ble_gap_unpair(&peer.ble_addr()) };
                if rc != 0 {
                    error!("BT profiles: can't remove the bond of {}: {}", format(&peer.addr), rc);
                }
                Verdict::Reject
            }
            None => {
                info!("BT profile {} bound to {}", self.active + 1, format(&peer.addr));
                self.profiles[self.active].host = Some(peer);
                self.save();
                Verdict::Accept
            }
        }
    }

    /// Profile bound to `host`.
    fn profile_of(&self, host: Host) -> Option<usize> {
        self.profiles.iter().position(|profile| profile.host == Some(host))
    }

    /// Removes the bond of the host of profile `index` and gives the profile a new address.
    /// The bond goes from the store only, the connection is dropped by the caller.
    fn release(&mut self, index: usize) {
        if let Some(host) = self.profiles[index].host.take() {
            let rc = unsafe { esp_idf_sys::ble_store_util_delete_peer(&host.ble_addr()) };
            if rc != 0 {
                warn!("BT profiles: can't remove the bond of {}: {}", format(&host.addr), rc);
            }
        }
        self.profiles[index].reset();
    }

    /// Bonds and profiles must match: bonds of unknown hosts are removed, hosts without bonds forgotten.
    fn sync_bonds(&mut self) {
        let mut peers = [ble_addr_t::default(); esp_idf_sys::MYNEWT_VAL_BLE_STORE_MAX_BONDS as usize];
        let mut count = 0;
        let rc = unsafe {
            esp_idf_sys::ble_store_util_bonded_peers(peers.as_mut_ptr(), &mut count, peers.len() as i32)
        };
        if rc != 0 {
            error!("BT profiles: can't read the bonds: {}", rc);
            return;
        }
        let bonded: Vec<Host> = peers[..count as usize].iter().map(Host::new).collect();

        for host in &bonded {
            if !self.profiles.iter().any(|profile| profile.host == Some(*host)) {
                info!("BT profiles: removing the bond of {}, no profile has it", format(&host.addr));
                let rc = unsafe { esp_idf_sys::ble_gap_unpair(&host.ble_addr()) };
                if rc != 0 {
                    error!("BT profiles: can't remove the bond of {}: {}", format(&host.addr), rc);
                }
            }
        }
        for (index, profile) in self.profiles.iter_mut().enumerate() {
            if let Some(host) = profile.host.filter(|host| !bonded.contains(host)) {
                info!("BT profile {}: {} has no bond, released", index + 1, format(&host.addr));
                profile.reset();
            }
        }
    }

    fn save(&mut self) {
        let Some(nvs) = &self.nvs else {
            return;
        };
        let blob = self.encode();
        if self.saved == Some(blob) {
            return;
        }
        match write_blob(nvs, &blob) {
            Ok(()) => self.saved = Some(blob),
            Err(e) => error!("BT profiles: can't save: {}", e),
        }
    }

    fn encode(&self) -> [u8; BLOB_LEN] {
        let mut blob = [0; BLOB_LEN];
        blob[0] = FORMAT;
        blob[1] = self.active as u8;
        for (profile, out) in self.profiles.iter().zip(blob[2..].chunks_exact_mut(PROFILE_LEN)) {
            out[..6].copy_from_slice(&profile.address);
            match profile.host {
                Some(host) => {
                    out[6] = host.addr_type;
                    out[7..].copy_from_slice(&host.addr);
                }
                None => out[6] = NO_HOST,
            }
        }
        blob
    }
}

/// Saved profiles in `nvs`. A blob of another format is [`Saved::Nothing`]: it can't be used, and
/// stays unusable, so it's replaced.
fn read(nvs: &EspNvs<NvsDefault>) -> Saved {
    let mut buf = [0; BLOB_LEN];
    match nvs.get_blob(KEY, &mut buf) {
        Ok(Some(blob)) => match decode(blob) {
            Some((active, profiles)) => Saved::Profiles(active, profiles),
            None => Saved::Nothing,
        },
        Ok(None) => Saved::Nothing,
        // Longer than this format, or not a blob.
        Err(e) if [ESP_ERR_NVS_INVALID_LENGTH, ESP_ERR_NVS_TYPE_MISMATCH].contains(&e.code()) => {
            warn!("BT profiles: saved ones are of another format, ignored");
            Saved::Nothing
        }
        Err(e) => {
            error!("BT profiles: can't read NVS: {}", e);
            Saved::Unreadable
        }
    }
}

/// Peer whose pairing is refused. Never locked while calling NimBLE: [`read_store`] runs under its lock.
static REFUSED_PEER: Mutex<Option<Host>> = Mutex::new(None);
/// Store read callback of NimBLE, wrapped by [`read_store`].
static STORE_READ: OnceLock<unsafe extern "C" fn(c_int, *const ble_store_key, *mut ble_store_value) -> c_int> =
    OnceLock::new();

/// Lets [`refuse_pairing`] work. After NimBLE is initialized (it sets the store up), once.
pub fn guard_pairing() {
    unsafe {
        let Some(read) = esp_idf_sys::ble_hs_cfg.store_read_cb else {
            error!("BT profiles: NimBLE has no store, pairing can't be refused");
            return;
        };
        if STORE_READ.set(read).is_ok() {
            esp_idf_sys::ble_hs_cfg.store_read_cb = Some(read_store);
        }
    }
}

/// Refuses pairing with `peer` (identity address), `None` allows it again. For a connection that is being
/// dropped: a pairing request may still come before the link is down, and esp32-nimble would delete the
/// bond the peer has with another profile (`BLE_GAP_EVENT_REPEAT_PAIRING`). It can't be overridden, but
/// NimBLE first reads that bond from the store and ignores the pairing request if the read fails: the
/// read fails then, see [`read_store`].
pub fn refuse_pairing(peer: Option<&ble_addr_t>) {
    *REFUSED_PEER.lock().unwrap_or_else(|e| e.into_inner()) = peer.map(Host::new);
}

/// Store read callback: the bond of the peer of [`refuse_pairing`] can't be read. Doesn't matter for
/// encryption, that peer is being disconnected. Runs under the NimBLE host lock, so it doesn't call NimBLE.
unsafe extern "C" fn read_store(obj_type: c_int, key: *const ble_store_key, dst: *mut ble_store_value) -> c_int {
    if obj_type == esp_idf_sys::BLE_STORE_OBJ_TYPE_PEER_SEC as c_int {
        let refused = *REFUSED_PEER.lock().unwrap_or_else(|e| e.into_inner());
        if refused == Some(Host::new(&(*key).sec.peer_addr)) {
            return esp_idf_sys::BLE_HS_EREJECT as c_int;
        }
    }
    match STORE_READ.get() {
        Some(read) => read(obj_type, key, dst),
        None => esp_idf_sys::BLE_HS_ENOTSUP as c_int,
    }
}

/// Own static random address in use, `None` if there's none yet.
fn own_address() -> Option<[u8; 6]> {
    let mut address = [0u8; 6];
    let rc = unsafe {
        esp_idf_sys::ble_hs_id_copy_addr(esp_idf_sys::BLE_ADDR_RANDOM as u8, address.as_mut_ptr(), core::ptr::null_mut())
    };
    (rc == 0).then_some(address)
}

/// Replaces the blob at once: `EspNvs::set_blob` removes the old one first, a power loss would lose both.
fn write_blob(nvs: &EspNvs<NvsDefault>, blob: &[u8]) -> Result<(), EspError> {
    let key = CString::new(KEY).unwrap();
    unsafe {
        esp!(esp_idf_sys::nvs_set_blob(nvs.handle(), key.as_ptr(), blob.as_ptr() as *const c_void, blob.len()))?;
        esp!(esp_idf_sys::nvs_commit(nvs.handle()))
    }
}

/// `(active, profiles)` of a saved blob, `None` if it's of another format or broken.
fn decode(blob: &[u8]) -> Option<(usize, [Profile; PROFILES])> {
    if blob.len() != BLOB_LEN || blob[0] != FORMAT || blob[1] as usize >= PROFILES {
        warn!("BT profiles: saved ones are of another format, ignored");
        return None;
    }
    let mut profiles = [Profile { address: [0; 6], host: None }; PROFILES];
    for (profile, data) in profiles.iter_mut().zip(blob[2..].chunks_exact(PROFILE_LEN)) {
        profile.address.copy_from_slice(&data[..6]);
        profile.host = (data[6] != NO_HOST).then(|| Host { addr_type: data[6], addr: data[7..].try_into().unwrap() });
    }
    Some((blob[1] as usize, profiles))
}

/// New static random address: the two top bits set, the rest random, not all ones (Core 6.B.1.3.2.1).
fn random_static_address() -> [u8; 6] {
    loop {
        let mut address = [0u8; 6];
        unsafe {
            esp_idf_sys::esp_fill_random(address.as_mut_ptr() as *mut c_void, address.len());
        }
        address[5] |= 0xC0;
        if address != [0xFF; 6] && address[..5] != [0; 5] {
            return address;
        }
    }
}

/// `AA:BB:CC:DD:EE:FF`, most significant byte first.
fn format(address: &[u8; 6]) -> String {
    let bytes: Vec<String> = address.iter().rev().map(|b| format!("{b:02X}")).collect();
    bytes.join(":")
}
