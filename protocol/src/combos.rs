//! Special combinations ([`Combo`]s that aren't chords): pure logic, like [`crate::keymap`]. The firmware feeds
//! [`Combos`] with the key changes of the keymap ([`Keymap::take_changes`](crate::keymap::Keymap::take_changes))
//! and runs the actions that fire, see [`Combos::poll`]. Chords are the keymap's.
//!
//! A combination watches the keys without changing what they do: presses count only while the held keys of the
//! combination are held, any other matrix key or releasing a held key breaks the combination. A joystick button
//! that isn't part of the combination breaks it when released, holding it is ignored (it may be a drag).
//! Once the held keys are held and the others pressed their number of times, the action fires.

use alloc::vec::Vec;

use crate::board::recovery_combo;
use crate::config::{Combo, ComboAction, ComboKey, ComboKeyIdent};
use crate::keymap::Change;

/// What [`Combos::poll`] found, in the order of the combinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboEvent {
    /// The presses left of a combination with a number of presses changed: `left` more and it fires.
    Progress { action: ComboAction, left: u8 },
    /// The combination is complete, its action is to be run.
    Fired(ComboAction),
}

/// Every special combination of a config, and the built-in [`recovery_combo`] after them, against the same
/// key changes.
#[derive(Debug, Clone)]
pub struct Combos {
    combinations: Vec<Combination>,
    /// Pressed keys, as their presses came.
    held: Vec<Change>,
}

impl Combos {
    /// Combinations of `combos` but the chords, then the recovery one.
    pub fn new(combos: &[Combo]) -> Self {
        let recovery = recovery_combo();
        let combinations = combos
            .iter()
            .filter(|combo| !combo.is_chord())
            .chain(core::iter::once(&recovery))
            .map(Combination::new)
            .collect();
        Self { combinations, held: Vec::new() }
    }

    /// Gets every press and release once, see [`Combos::reset`] for the keys held meanwhile.
    pub fn changed(&mut self, change: &Change) {
        if change.pressed {
            self.held.push(*change);
        } else {
            self.held.retain(|held| held.key != change.key);
        }
        for combination in &mut self.combinations {
            combination.changed(change);
        }
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
    pub fn reset(&mut self) {
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
        Self { action: combo.action, keys: combo.keys.iter().map(Key::new).collect(), invalid: false, left: None }
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
        if self.invalid {
            self.reset(held);
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
            events.push(ComboEvent::Fired(self.action));
        }
    }

    fn reset(&mut self, held: &[Change]) {
        for key in &mut self.keys {
            key.pressed = false;
            key.count = 0;
        }
        self.invalid = false;
        self.left = None;

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

    fn change(key: (u8, u8), pressed: bool) -> Change {
        Change { key: KeyId::Matrix { x: key.0, y: key.1 }, action: KeyAction::None, pressed }
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
        let mut combos = Combos::new(&[fn_q_twice()]);
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
        let mut combos = Combos::new(&[fn_q_twice()]);
        assert_eq!(fired(&tap(&mut combos, Q)), []);
        combos.changed(&change(FN, true));
        // The tap before Fn broke it, the reset starts over with Fn held.
        assert_eq!(fired(&combos.poll()), []);
        tap(&mut combos, Q);
        assert_eq!(fired(&tap(&mut combos, Q)), [ComboAction::BtSelect(1)]);
    }

    #[test]
    fn other_key_or_release_breaks_it() {
        let mut combos = Combos::new(&[fn_q_twice()]);
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
        let button = |pressed| Change { key: KeyId::Joystick(JoystickButton::Left), action: KeyAction::None, pressed };
        let mut combos = Combos::new(&[fn_q_twice()]);
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
        let mut combos = Combos::new(&[combo]);
        combos.changed(&Change { key: KeyId::Matrix { x: 0, y: 4 }, action: ctrl, pressed: true });
        let a = |pressed| Change { key: KeyId::Matrix { x: 0, y: 2 }, action: KeyAction::Key(HidKey::A), pressed };
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
        let mut combos = Combos::new(&[chord]);
        combos.changed(&change(Q, true));
        combos.changed(&change(W, true));
        assert_eq!(combos.poll(), []);

        let mut combos = Combos::new(&[]);
        combos.changed(&change(FN, true));
        combos.changed(&change((0, 4), true));
        let events: Vec<ComboEvent> = (0..5).flat_map(|_| tap(&mut combos, (0, 0))).collect();
        assert_eq!(fired(&events), [ComboAction::ResetConfig]);
        assert_eq!(events[0], ComboEvent::Progress { action: ComboAction::ResetConfig, left: 4 });
    }

    #[test]
    fn reset_drops_the_progress() {
        let mut combos = Combos::new(&[fn_q_twice()]);
        combos.changed(&change(FN, true));
        tap(&mut combos, Q);
        combos.reset();
        // Fn is still held.
        assert_eq!(tap(&mut combos, Q), [ComboEvent::Progress { action: ComboAction::BtSelect(1), left: 1 }]);
    }
}
