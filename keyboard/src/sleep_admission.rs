//! Coordinates deep sleep with asynchronous BLE, config, and firmware work.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub(crate) static SLEEP: SleepCoordinator = SleepCoordinator::new();
const WORK_RETRY: Duration = Duration::from_secs(1);

pub(crate) struct SleepCoordinator {
    // When both locks are held, the participant lock comes first.
    state: Mutex<State>,
}

struct State {
    host: HostState,
    session_expires_at: Option<Instant>,
    reservations: usize,
    committed: bool,
}

pub(crate) enum HostState {
    Unavailable,
    PairingUntil(Instant),
    Ready,
}

/// Both host deadlines use the same idle start, even if the host changes while we wait.
#[derive(Clone, Copy)]
pub(crate) struct IdlePeriod {
    since: Instant,
    connected: Duration,
    disconnected: Duration,
}

impl IdlePeriod {
    pub(crate) fn new(since: Instant, connected: Duration, disconnected: Duration) -> Self {
        Self { since, connected, disconnected }
    }

    fn sleep_at(self, state: &State) -> Instant {
        let deadline = match state.host {
            HostState::Unavailable => self.since + self.disconnected,
            HostState::PairingUntil(until) => (self.since + self.disconnected).max(until),
            HostState::Ready => self.since + self.connected,
        };
        state.session_expires_at.map_or(deadline, |expiry| expiry.max(deadline))
    }
}

/// Keeps deep sleep from committing until admitted work finishes, including boot verification.
pub(crate) struct SleepReservation<'a> {
    coordinator: &'a SleepCoordinator,
}

/// The final check holds the coordinator lock through the GPIO check and publication.
pub(crate) struct SleepCommit<'a> {
    state: MutexGuard<'a, State>,
}

impl SleepCoordinator {
    const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                host: HostState::Unavailable,
                session_expires_at: None,
                reservations: 0,
                committed: false,
            }),
        }
    }

    pub(crate) fn reserve(&self) -> Option<SleepReservation<'_>> {
        let mut state = self.state.lock().unwrap();
        if state.committed {
            return None;
        }
        state.reservations += 1;
        Some(SleepReservation { coordinator: self })
    }

    /// Snapshot for waiting and reversible preparation. All facts are rechecked at commit.
    pub(crate) fn check(&self, idle: IdlePeriod, now: Instant) -> Result<(), Instant> {
        self.prepare_commit(idle, now).map(drop)
    }

    /// Returns a retry deadline or grants the main task the right to publish sleep commitment.
    pub(crate) fn prepare_commit(&self, idle: IdlePeriod, now: Instant) -> Result<SleepCommit<'_>, Instant> {
        let retry = now + WORK_RETRY;
        let state = self.state.try_lock().map_err(|_| retry)?;
        let at = idle.sleep_at(&state);
        if at > now {
            return Err(at);
        }
        if state.committed || state.reservations != 0 {
            return Err(retry);
        }
        Ok(SleepCommit { state })
    }
}

impl SleepCommit<'_> {
    pub(crate) fn publish(&mut self) {
        self.state.committed = true;
    }
}

impl SleepReservation<'_> {
    /// Extends the reservation to additional work required by the admitted operation.
    pub(crate) fn retain(&self) -> Self {
        self.coordinator.state.lock().unwrap().reservations += 1;
        Self { coordinator: self.coordinator }
    }

    /// Publish while the link serializes connection state changes.
    pub(crate) fn set_host_state(&self, host: HostState) {
        self.coordinator.state.lock().unwrap().host = host;
    }

    /// Publish while the config service serializes session changes.
    pub(crate) fn set_session_expiry(&self, expires_at: Option<Instant>) {
        self.coordinator.state.lock().unwrap().session_expires_at = expires_at;
    }
}

impl Drop for SleepReservation<'_> {
    fn drop(&mut self) {
        self.coordinator.state.lock().unwrap().reservations -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread};

    const CONNECTED: Duration = Duration::from_secs(120);
    const DISCONNECTED: Duration = Duration::from_secs(30);

    #[test]
    fn late_queued_work_blocks_commit_until_all_reservations_drop() {
        let coordinator = &SLEEP;
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        let now = start + DISCONNECTED;
        assert!(coordinator.check(idle, now).is_ok());
        let (tx, rx) = mpsc::channel();
        tx.send(coordinator.reserve().unwrap()).unwrap();
        assert_eq!(coordinator.check(idle, now), Err(now + WORK_RETRY));
        assert!(coordinator.prepare_commit(idle, now).is_err());
        let running = rx.recv().unwrap();
        let retained = running.retain();
        drop(running);
        assert!(coordinator.prepare_commit(idle, now).is_err());
        drop(retained);
        assert!(coordinator.check(idle, now).is_ok());
    }

    #[test]
    fn host_changes_recalculate_from_the_same_idle_start() {
        let coordinator = SleepCoordinator::new();
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        assert_eq!(coordinator.check(idle, start), Err(start + DISCONNECTED));
        let now = start + DISCONNECTED;
        assert!(coordinator.check(idle, now).is_ok());
        coordinator.reserve().unwrap().set_host_state(HostState::Ready);
        assert_eq!(coordinator.prepare_commit(idle, now).err(), Some(start + CONNECTED));
        coordinator.reserve().unwrap().set_host_state(HostState::Unavailable);
        assert!(coordinator.check(idle, now).is_ok());
    }

    #[test]
    fn late_pairing_postpones_commit_until_its_deadline_or_disconnect() {
        let coordinator = SleepCoordinator::new();
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        let now = start + DISCONNECTED;
        let pairing_until = now + Duration::from_secs(14);
        assert!(coordinator.check(idle, now).is_ok());

        coordinator.reserve().unwrap().set_host_state(HostState::PairingUntil(pairing_until));
        assert_eq!(coordinator.prepare_commit(idle, now).err(), Some(pairing_until));
        assert!(coordinator.check(idle, pairing_until).is_ok());

        coordinator.reserve().unwrap().set_host_state(HostState::Unavailable);
        assert!(coordinator.check(idle, now).is_ok());
    }

    #[test]
    fn commit_rechecks_session_expiry_after_preparation() {
        let coordinator = SleepCoordinator::new();
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        let now = start + DISCONNECTED;
        assert!(coordinator.check(idle, now).is_ok());
        let expiry = now + Duration::from_secs(5);
        coordinator.reserve().unwrap().set_session_expiry(Some(expiry));
        assert_eq!(coordinator.prepare_commit(idle, now).err(), Some(expiry));
        assert!(coordinator.check(idle, expiry).is_ok());
        let renewed = expiry + Duration::from_secs(3);
        coordinator.reserve().unwrap().set_session_expiry(Some(renewed));
        assert_eq!(coordinator.prepare_commit(idle, expiry).err(), Some(renewed));
        coordinator.reserve().unwrap().set_session_expiry(None);
        assert!(coordinator.check(idle, expiry).is_ok());
    }

    #[test]
    fn boot_verification_reservation_blocks_until_confirmation() {
        let coordinator = SleepCoordinator::new();
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        let verification = coordinator.reserve().unwrap();
        let now = start + CONNECTED;
        assert_eq!(coordinator.check(idle, now), Err(now + WORK_RETRY));
        drop(verification);
        assert!(coordinator.check(idle, now).is_ok());
    }

    #[test]
    fn canceling_commit_allows_work() {
        let coordinator = SleepCoordinator::new();
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        let now = start + DISCONNECTED;
        drop(coordinator.prepare_commit(idle, now).unwrap());
        assert!(coordinator.reserve().is_some());
    }

    #[test]
    fn commitment_rejects_a_waiting_publisher() {
        let coordinator = SleepCoordinator::new();
        let start = Instant::now();
        let idle = IdlePeriod::new(start, CONNECTED, DISCONNECTED);
        let now = start + DISCONNECTED;
        let mut commit = coordinator.prepare_commit(idle, now).unwrap();
        let coordinator = &coordinator;
        thread::scope(|scope| {
            let (tx, rx) = mpsc::channel();
            let publisher = scope.spawn(move || {
                tx.send(()).unwrap();
                coordinator.reserve().is_some()
            });
            rx.recv().unwrap();
            commit.publish();
            drop(commit);
            assert!(!publisher.join().unwrap());
        });
        assert!(coordinator.prepare_commit(idle, now).is_err());
    }
}
