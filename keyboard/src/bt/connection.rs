use std::time::{Duration, Instant};

/// Identifies one physical connection even when NimBLE reuses a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionId {
    pub(super) handle: u16,
    generation: u64,
}

/// A host not accepted this long after CONNECT would hold the only connection.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Time for the release reports to reach the old host on a profile switch.
const RELEASE_DELAY: Duration = Duration::from_millis(50);

enum Phase {
    Pairing(Instant),
    /// Authorization was claimed in time. Profile validation and storage may take longer.
    Authenticating,
    Ready,
    Dropping(Option<Instant>),
}

struct Host {
    id: ConnectionId,
    phase: Phase,
}

#[derive(Default)]
pub(super) struct Connection {
    generation: u64,
    host: Option<Host>,
}

impl Connection {
    pub(super) fn id(&self) -> Option<ConnectionId> {
        self.host.as_ref().map(|host| host.id)
    }

    pub(super) fn ready(&self) -> Option<ConnectionId> {
        let host = self.host.as_ref()?;
        matches!(host.phase, Phase::Ready).then_some(host.id)
    }

    pub(super) fn ready_handle_for(&self, id: ConnectionId) -> Option<u16> {
        (self.ready() == Some(id)).then_some(id.handle)
    }

    /// Returns the acceptance deadline if this connection can pair.
    pub(super) fn connected(&mut self, handle: u16, valid: bool, now: Instant) -> Option<Instant> {
        self.generation = self.generation.checked_add(1).expect("BLE connection generation exhausted");
        let id = ConnectionId { handle, generation: self.generation };
        let pairing_until = valid.then(|| now + ACCEPT_TIMEOUT);
        let phase = pairing_until.map_or(Phase::Dropping(Some(now)), Phase::Pairing);
        self.host = Some(Host { id, phase });
        pairing_until
    }

    pub(super) fn claim_authentication(&mut self, handle: u16, now: Instant) -> Option<ConnectionId> {
        let host = self.host.as_mut().filter(|host| host.id.handle == handle)?;
        if !matches!(host.phase, Phase::Pairing(deadline) if now < deadline) {
            return None;
        }
        host.phase = Phase::Authenticating;
        Some(host.id)
    }

    pub(super) fn finish_authentication(&mut self, claim: ConnectionId, accepted: bool, now: Instant) -> bool {
        let Some(host) = self.host.as_mut().filter(|host| host.id == claim) else {
            return false;
        };
        if !matches!(host.phase, Phase::Authenticating) {
            return false;
        }
        host.phase = if accepted { Phase::Ready } else { Phase::Dropping(Some(now)) };
        accepted
    }

    pub(super) fn disconnected(&mut self, handle: u16) -> Option<ConnectionId> {
        let id = self.id().filter(|id| id.handle == handle)?;
        self.host = None;
        Some(id)
    }

    pub(super) fn drop_host(&mut self, now: Instant) -> bool {
        let Some(host) = self.host.as_mut() else {
            return false;
        };
        if !matches!(host.phase, Phase::Dropping(_)) {
            let delay = if matches!(host.phase, Phase::Ready) { RELEASE_DELAY } else { Duration::ZERO };
            host.phase = Phase::Dropping(Some(now + delay));
        }
        true
    }

    pub(super) fn disconnect_at(&self) -> Option<Instant> {
        match self.host.as_ref()?.phase {
            Phase::Pairing(deadline) => Some(deadline),
            Phase::Authenticating | Phase::Ready => None,
            Phase::Dropping(deadline) => deadline,
        }
    }

    /// Returns the disconnect target and whether pairing timed out.
    pub(super) fn begin_disconnect(&mut self, now: Instant) -> Option<(ConnectionId, bool)> {
        if self.disconnect_at()? > now {
            return None;
        }
        let host = self.host.as_mut()?;
        let pairing = matches!(host.phase, Phase::Pairing(_));
        host.phase = Phase::Dropping(None);
        Some((host.id, pairing))
    }

    pub(super) fn retry_disconnect(&mut self, id: ConnectionId, at: Instant) {
        if let Some(host) = self.host.as_mut().filter(|host| host.id == id && matches!(host.phase, Phase::Dropping(None))) {
            host.phase = Phase::Dropping(Some(at));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_claimed_before_timeout_can_finish_afterward() {
        let mut connection = Connection::default();
        let now = Instant::now();
        let deadline = now + ACCEPT_TIMEOUT;
        connection.connected(1, true, now);
        let claim = connection.claim_authentication(1, deadline - Duration::from_millis(1)).unwrap();
        assert!(connection.claim_authentication(1, deadline).is_none());
        assert!(connection.begin_disconnect(deadline).is_none());
        assert!(connection.finish_authentication(claim, true, deadline));
        assert_eq!(connection.ready(), Some(claim));
    }

    #[test]
    fn pairing_timeout_excludes_authentication() {
        let mut connection = Connection::default();
        let now = Instant::now();
        let deadline = now + ACCEPT_TIMEOUT;
        assert_eq!(connection.connected(1, true, now), Some(deadline));
        assert!(connection.claim_authentication(1, deadline).is_none());
        assert_eq!(connection.begin_disconnect(deadline), Some((connection.id().unwrap(), true)));
        assert!(connection.claim_authentication(1, deadline).is_none());
        assert!(connection.ready().is_none());
    }

    #[test]
    fn rejected_connection_has_no_pairing_deadline() {
        let mut connection = Connection::default();
        let now = Instant::now();
        assert_eq!(connection.connected(1, false, now), None);
        assert_eq!(connection.begin_disconnect(now), Some((connection.id().unwrap(), false)));
    }

    #[test]
    fn reused_handle_gets_a_new_identity() {
        let mut connection = Connection::default();
        let now = Instant::now();
        connection.connected(1, true, now);
        let old = connection.claim_authentication(1, now).unwrap();
        assert_eq!(connection.disconnected(1), Some(old));
        connection.connected(1, true, now);
        let new = connection.claim_authentication(1, now).unwrap();
        assert_ne!(new, old);
        assert!(!connection.finish_authentication(old, true, now));
        assert!(connection.finish_authentication(new, true, now));
        connection.retry_disconnect(old, now);
        assert!(connection.begin_disconnect(now).is_none());
        assert_eq!(connection.ready(), Some(new));
    }

    #[test]
    fn replies_cannot_reach_a_replacement_connection_with_the_same_handle() {
        let mut connection = Connection::default();
        let now = Instant::now();
        let handle = 1;
        connection.connected(handle, true, now);
        let old = connection.claim_authentication(handle, now).unwrap();
        assert!(connection.finish_authentication(old, true, now));
        assert_eq!(connection.ready_handle_for(old), Some(handle));
        connection.disconnected(handle);
        assert_eq!(connection.ready_handle_for(old), None);

        connection.connected(handle, true, now);
        let new = connection.claim_authentication(handle, now).unwrap();
        assert!(connection.finish_authentication(new, true, now));
        assert_eq!(connection.ready_handle_for(old), None);
        assert_eq!(connection.ready_handle_for(new), Some(handle));
    }

    #[test]
    fn profile_switch_invalidates_a_ready_identity_before_release_delay() {
        let mut connection = Connection::default();
        let now = Instant::now();
        connection.connected(1, true, now);
        let id = connection.claim_authentication(1, now).unwrap();
        assert!(connection.finish_authentication(id, true, now));
        assert!(connection.drop_host(now));
        assert!(connection.ready().is_none());
        assert!(connection.begin_disconnect(now).is_none());
        assert_eq!(connection.begin_disconnect(now + RELEASE_DELAY), Some((id, false)));
    }
}
