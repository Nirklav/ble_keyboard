//! Special combinations ([`Combo`]s that aren't chords): pure logic, like [`crate::keymap`]. The firmware feeds
//! [`Combos`] with the key changes of the keymap ([`Keymap::take_changes`](crate::keymap::Keymap::take_changes))
//! and runs the actions that fire, see [`Combos::poll`]. Chords are the keymap's.
//!
//! A combination watches the keys without changing what they do: presses count only while the held keys of the
//! combination are held, any other matrix key or releasing a held key breaks the combination. A joystick button
//! that isn't part of the combination breaks it when released, holding it is ignored (it may be a drag).
//! Once the held keys are held and the others pressed their number of times, the action fires.
//! No press for longer than [`Settings::combo_timeout_ms`] after a release drops the progress, see
//! [`Combos::tick`]; a key held down meanwhile is no pause.

use alloc::vec::Vec;

use crate::board::recovery_combo;
use crate::config::{Combo, ComboAction, ComboKey, ComboKeyIdent, Settings};
use crate::keymap::Change;

/// What [`Combos::poll`] found, in the order of the combinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboEvent {
    /// The presses left of a combination with a number of presses changed: `left` more and it fires.
    Progress { action: ComboAction, left: u8 },
    /// The combination is complete, its action is to be run.
    Fired(ComboAction),
    /// The progress of a combination broke or timed out, `left` was in its last [`ComboEvent::Progress`].
    Dropped { action: ComboAction, left: u8 },
}

/// Every special combination of a config, and the built-in [`recovery_combo`] after them, against the same
/// key changes.
#[derive(Debug, Clone)]
pub struct Combos {
    combinations: Vec<Combination>,
    /// Pressed keys, as their presses came.
    held: Vec<Change>,
    /// [`Settings::combo_timeout_ms`].
    timeout: u32,
    /// Time of the last release while no key was pressed after it: the pause of the timeout.
    released: Option<u32>,
}

impl Combos {
    /// Combinations of `combos` but the chords, then the recovery one.
    pub fn new(combos: &[Combo], settings: &Settings) -> Self {
        let recovery = recovery_combo();
        let combinations = combos
            .iter()
            .filter(|combo| !combo.is_chord())
            .chain(core::iter::once(&recovery))
            .map(Combination::new)
            .collect();
        Self { combinations, held: Vec::new(), timeout: settings.combo_timeout_ms.into(), released: None }
    }

    /// Gets every press and release once, in order. A press after the timeout drops the progress first, like
    /// [`Combos::tick`] would have.
    pub fn changed(&mut self, change: &Change) {
        if change.pressed {
            self.tick(change.time);
            self.released = None;
            self.held.push(*change);
        } else {
            self.released = Some(change.time);
            self.held.retain(|held| held.key != change.key);
        }
        for combination in &mut self.combinations {
            combination.changed(change);
        }
    }

    /// At `now` in ms: once the timeout passed after a release with no press, every combination starts over,
    /// see [`Combos::reset`]. Not while a key change waits in the keymap, it may have come in time.
    pub fn tick(&mut self, now: u32) {
        // Signed: a decided chord press comes after the release of an earlier key that came later.
        if self.released.is_some_and(|released| now.wrapping_sub(released) as i32 > self.timeout as i32) {
            self.reset();
        }
    }

    /// Ms from `now` until [`Combos::tick`] drops a progress; `None` if there is no progress or the timeout
    /// doesn't run (a key is pressed).
    pub fn timeout_in(&self, now: u32) -> Option<u32> {
        let released = self.released?;
        let progress = self.combinations.iter().any(|combination| combination.left.is_some());
        progress.then(|| (self.timeout + 1).saturating_sub(now.wrapping_sub(released)))
    }

    /// After the changes of a scan: the progress and the actions that fired. A broken or fired combination
    /// starts over, see [`Combos::reset`].
    pub fn poll(&mut self) -> Vec<ComboEvent> {
        let mut events = Vec::new();
        for combination in &mut self.combinations {
            combination.poll(&self.held, &mut events);
        }
        events
    }

    /// All progress is lost, every combination starts over with the held keys pressed again: a held key of
    /// a combination still counts as held, a held matrix key outside it breaks the new attempt at once.
    /// The next [`Combos::poll`] tells the progress shown is dropped.
    pub fn reset(&mut self) {
        self.released = None;
        for combination in &mut self.combinations {
            combination.reset(&self.held);
        }
    }
}

#[derive(Debug, Clone)]
struct Combination {
    action: ComboAction,
    keys: Vec<Key>,
    invalid: bool,
    /// Presses left in the last [`ComboEvent::Progress`], so it's sent only when they change.
    left: Option<u8>,
    /// `left` of a progress a reset dropped, for [`ComboEvent::Dropped`].
    dropped: Option<u8>,
}

#[derive(Debug, Clone)]
struct Key {
    ident: ComboKeyIdent,
    pressed: bool,
    /// Presses it needs, 0 if it's held.
    required: u8,
    count: u8,
}

impl Key {
    fn new(key: &ComboKey) -> Self {
        Self { ident: key.ident, pressed: false, required: key.count, count: 0 }
    }
}

impl Combination {
    fn new(combo: &Combo) -> Self {
        Self {
            action: combo.action,
            keys: combo.keys.iter().map(Key::new).collect(),
            invalid: false,
            left: None,
            dropped: None,
        }
    }

    fn changed(&mut self, change: &Change) {
        let coord = change.key.coord();
        let holding = self.holding();
        let mut any_matched = false;

        for key in &mut self.keys {
            if !key.ident.matches(coord, &change.action) {
                continue;
            }
            any_matched = true;

            if change.pressed {
                key.pressed = true;
            } else if key.pressed {
                key.pressed = false;
                if key.required == 0 || !holding {
                    self.invalid = true;
                } else {
                    key.count = key.count.saturating_add(1);
                }
            }
        }

        // A joystick button breaks it only once clicked, while held it may be a drag.
        if !any_matched && (coord.is_some() || !change.pressed) {
            self.invalid = true;
        }
    }

    fn holding(&self) -> bool {
        self.keys.iter().all(|key| key.required > 0 || key.pressed)
    }

    fn poll(&mut self, held: &[Change], events: &mut Vec<ComboEvent>) {
        let invalid = self.invalid;
        if invalid {
            self.reset(held);
        }
        if let Some(left) = self.dropped.take() {
            events.push(ComboEvent::Dropped { action: self.action, left });
        }
        if invalid {
            return;
        }

        let matched = self.holding() && self.keys.iter().all(|key| key.count >= key.required);

        let left = self
            .keys
            .iter()
            .filter(|key| key.required > 0 && key.count > 0)
            .map(|key| key.required.saturating_sub(key.count))
            .next_back();
        if let Some(left) = left.filter(|_| left != self.left) {
            self.left = Some(left);
            events.push(ComboEvent::Progress { action: self.action, left });
        }

        if matched {
            self.reset(held);
            // Not dropped, done.
            self.dropped = None;
            events.push(ComboEvent::Fired(self.action));
        }
    }

    fn reset(&mut self, held: &[Change]) {
        for key in &mut self.keys {
            key.pressed = false;
            key.count = 0;
        }
        self.invalid = false;
        if let Some(left) = self.left.take() {
            self.dropped = Some(left);
        }

        for change in held {
            self.changed(change);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::config::{JoystickButton, KeyAction};
    use crate::keymap::KeyId;
    use crate::keys::{Key as HidKey, KeyMod};

    const FN: (u8, u8) = (2, 4);
    const Q: (u8, u8) = (0, 1);
    const W: (u8, u8) = (1, 1);

    fn coord((x, y): (u8, u8)) -> ComboKeyIdent {
        ComboKeyIdent::Coord { x, y }
    }

    /// Hold Fn, press Q twice: selects BLE profile 1.
    fn fn_q_twice() -> Combo {
        Combo {
            keys: vec![ComboKey { ident: coord(FN), count: 0 }, ComboKey { ident: coord(Q), count: 2 }],
            action: ComboAction::BtSelect(1),
        }
    }

    const TIMEOUT: u32 = 1000;

    fn change(key: (u8, u8), pressed: bool) -> Change {
        at(key, pressed, 0)
    }

    fn at(key: (u8, u8), pressed: bool, time: u32) -> Change {
        Change { key: KeyId::Matrix { x: key.0, y: key.1 }, action: KeyAction::None, pressed, time }
    }

    fn tap(combos: &mut Combos, key: (u8, u8)) -> Vec<ComboEvent> {
        combos.changed(&change(key, true));
        let mut events = combos.poll();
        combos.changed(&change(key, false));
        events.extend(combos.poll());
        events
    }

    fn fired(events: &[ComboEvent]) -> Vec<ComboAction> {
        events.iter().filter_map(|event| if let ComboEvent::Fired(action) = event { Some(*action) } else { None }).collect()
    }

    #[test]
    fn fires_with_progress() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&change(FN, true));
        assert_eq!(combos.poll(), []);
        assert_eq!(tap(&mut combos, Q), [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 }]);
        assert_eq!(
            tap(&mut combos, Q),
            [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 0 }, ComboEvent::Fired(ComboAction::BtSelect(1))]
        );
        // Starts over with Fn still held.
        assert_eq!(tap(&mut combos, Q), [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 }]);
        assert_eq!(fired(&tap(&mut combos, Q)), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn presses_count_only_while_held() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        assert_eq!(fired(&tap(&mut combos, Q)), []);
        combos.changed(&change(FN, true));
        // The tap before Fn broke it, the reset starts over with Fn held.
        assert_eq!(fired(&combos.poll()), []);
        tap(&mut combos, Q);
        assert_eq!(fired(&tap(&mut combos, Q)), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn other_key_or_release_breaks_it() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&change(FN, true));
        tap(&mut combos, Q);
        tap(&mut combos, W);
        assert_eq!(fired(&tap(&mut combos, Q)), []);
        assert_eq!(fired(&tap(&mut combos, Q)), [ComboAction::BtSelect(1)]);

        tap(&mut combos, Q);
        combos.changed(&change(FN, false));
        combos.poll();
        combos.changed(&change(FN, true));
        tap(&mut combos, Q);
        assert_eq!(fired(&combos.poll()), []);
        assert_eq!(fired(&tap(&mut combos, Q)), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn joystick_button_breaks_it_once_clicked() {
        let button = |pressed| Change { key: KeyId::Joystick(JoystickButton::Left), action: KeyAction::None, pressed, time: 0 };
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&change(FN, true));
        tap(&mut combos, Q);
        combos.changed(&button(true));
        assert_eq!(fired(&tap(&mut combos, Q)), [ComboAction::BtSelect(1)]);

        tap(&mut combos, Q);
        combos.changed(&button(false));
        combos.poll();
        assert_eq!(fired(&tap(&mut combos, Q)), []);
    }

    #[test]
    fn matches_by_action() {
        let ctrl = KeyAction::Mod(KeyMod::LeftCtrl);
        let combo = Combo {
            keys: vec![
                ComboKey { ident: ComboKeyIdent::Action(ctrl), count: 0 },
                ComboKey { ident: ComboKeyIdent::Action(KeyAction::Key(HidKey::A)), count: 1 },
            ],
            action: ComboAction::SwitchLayout,
        };
        let mut combos = Combos::new(&[combo], &Settings::default());
        combos.changed(&Change { key: KeyId::Matrix { x: 0, y: 4 }, action: ctrl, pressed: true, time: 0 });
        let a = |pressed| Change { key: KeyId::Matrix { x: 0, y: 2 }, action: KeyAction::Key(HidKey::A), pressed, time: 0 };
        combos.changed(&a(true));
        combos.changed(&a(false));
        assert_eq!(fired(&combos.poll()), [ComboAction::SwitchLayout]);
    }

    #[test]
    fn chords_are_left_out_and_recovery_is_built_in() {
        let chord = Combo {
            keys: vec![ComboKey { ident: coord(Q), count: 0 }, ComboKey { ident: coord(W), count: 0 }],
            action: ComboAction::Action(KeyAction::Key(HidKey::Escape)),
        };
        let mut combos = Combos::new(&[chord], &Settings::default());
        combos.changed(&change(Q, true));
        combos.changed(&change(W, true));
        assert_eq!(combos.poll(), []);

        let mut combos = Combos::new(&[], &Settings::default());
        combos.changed(&change(FN, true));
        combos.changed(&change((0, 4), true));
        let events: Vec<ComboEvent> = (0..5).flat_map(|_| tap(&mut combos, (0, 0))).collect();
        assert_eq!(fired(&events), [ComboAction::ResetConfig]);
        assert_eq!(events[0], ComboEvent::Progress { action: ComboAction::ResetConfig, left: 4 });
    }

    #[test]
    fn reset_drops_the_progress() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&change(FN, true));
        tap(&mut combos, Q);
        combos.reset();
        assert_eq!(combos.poll(), [ComboEvent::Dropped { action: ComboAction::BtSelect(1), left: 1 }]);
        // Fn is still held.
        assert_eq!(tap(&mut combos, Q), [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 }]);
    }

    #[test]
    fn pause_after_a_release_drops_the_progress() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        assert_eq!(Settings::default().combo_timeout_ms as u32, TIMEOUT);
        combos.changed(&at(FN, true, 0));
        combos.changed(&at(Q, true, 100));
        combos.changed(&at(Q, false, 200));
        assert_eq!(combos.poll(), [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 }]);
        assert_eq!(combos.timeout_in(700), Some(TIMEOUT + 1 - 500));
        combos.tick(200 + TIMEOUT);
        assert_eq!(combos.poll(), []);
        combos.tick(200 + TIMEOUT + 1);
        assert_eq!(combos.poll(), [ComboEvent::Dropped { action: ComboAction::BtSelect(1), left: 1 }]);
        assert_eq!(combos.timeout_in(2000), None);
        // Fn is still held, the late press counts as the first one.
        combos.changed(&at(Q, true, 2000));
        combos.changed(&at(Q, false, 2100));
        assert_eq!(combos.poll(), [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 }]);
        combos.changed(&at(Q, true, 2200));
        combos.changed(&at(Q, false, 2300));
        assert_eq!(fired(&combos.poll()), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn late_press_drops_the_progress_without_a_tick() {
        // A tap-hold key decided after the timeout: its press came late, the change says so.
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&at(FN, true, 0));
        combos.changed(&at(Q, true, 100));
        combos.changed(&at(Q, false, 200));
        combos.poll();
        combos.changed(&at(Q, true, 200 + TIMEOUT + 1));
        combos.changed(&at(Q, false, 300 + TIMEOUT));
        assert_eq!(
            combos.poll(),
            [
                ComboEvent::Dropped { action: ComboAction::BtSelect(1), left: 1 },
                ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 },
            ]
        );
    }

    #[test]
    fn holding_a_key_is_no_pause() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&at(FN, true, 0));
        combos.changed(&at(Q, true, 100));
        combos.changed(&at(Q, false, 200));
        combos.poll();
        // Held far longer than the timeout.
        combos.changed(&at(Q, true, 300));
        assert_eq!(combos.timeout_in(300), None);
        combos.tick(300 + 3 * TIMEOUT);
        combos.changed(&at(Q, false, 300 + 3 * TIMEOUT));
        assert_eq!(fired(&combos.poll()), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn broken_progress_is_dropped_and_fired_one_is_not() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&change(FN, true));
        tap(&mut combos, Q);
        assert_eq!(tap(&mut combos, W), [ComboEvent::Dropped { action: ComboAction::BtSelect(1), left: 1 }]);
        tap(&mut combos, Q);
        assert_eq!(
            tap(&mut combos, Q),
            [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 0 }, ComboEvent::Fired(ComboAction::BtSelect(1))]
        );
        assert_eq!(combos.poll(), []);
    }

    #[test]
    fn press_older_than_the_release_keeps_the_progress() {
        // A chord press is decided after the release of an earlier key, with its earlier time.
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        combos.changed(&at(FN, true, 0));
        combos.changed(&at(Q, true, 100));
        combos.changed(&at(Q, false, 210));
        combos.poll();
        combos.changed(&at(Q, true, 200));
        combos.changed(&at(Q, false, 300));
        assert_eq!(fired(&combos.poll()), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn timeout_is_a_setting() {
        let settings = Settings { combo_timeout_ms: 3000, ..Settings::default() };
        let mut combos = Combos::new(&[fn_q_twice()], &settings);
        combos.changed(&at(FN, true, 0));
        combos.changed(&at(Q, true, 100));
        combos.changed(&at(Q, false, 200));
        combos.poll();
        combos.tick(200 + 3000);
        combos.changed(&at(Q, true, 200 + 3000));
        combos.changed(&at(Q, false, 300 + 3000));
        assert_eq!(fired(&combos.poll()), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn pause_of_the_timeout_keeps_the_progress_across_the_wrap() {
        let mut combos = Combos::new(&[fn_q_twice()], &Settings::default());
        let start = u32::MAX - 100;
        combos.changed(&at(FN, true, start));
        combos.changed(&at(Q, true, start));
        combos.changed(&at(Q, false, start));
        combos.poll();
        // Exactly the timeout later, after the time wrapped: still the second press.
        let late = start.wrapping_add(TIMEOUT);
        combos.tick(late);
        combos.changed(&at(Q, true, late));
        combos.changed(&at(Q, false, late));
        assert_eq!(fired(&combos.poll()), [ComboAction::BtSelect(1)]);
    }
}
