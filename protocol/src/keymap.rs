//! Key processing: layers, transparent keys and tap-hold keys. Pure logic on millisecond timestamps,
//! the firmware feeds it with the scanned key edges and puts [`Keymap::report`] into the HID reports.
//!
//! Layers: a base layer ([`Keymap::set_base`], the first layout after boot) and a stack of layers on top
//! of it. A key does what it does in the topmost active layer, [`KeyAction::Transparent`] falls through
//! to the next one below, down to the base layer.
//! - [`KeyAction::Momentary`] (and [`KeyAction::Fn`], [`KeyAction::LayerTap`] held) pushes its layer
//!   on press and removes it on release;
//! - [`KeyAction::Toggle`] removes its layer from the stack if it's there, otherwise pushes it; so a toggle of
//!   a layer held by a momentary key turns it off, and releasing a momentary key removes the topmost entry
//!   of its layer, whoever pushed it;
//! - [`KeyAction::To`] makes its layer the base one and clears the stack.
//!
//! The action of a key is looked up when it's pressed and stays until it's released, whatever happens
//! to the layers meanwhile, so a release always releases what the press pressed.
//!
//! Tap-hold ([`KeyAction::ModTap`], [`KeyAction::LayerTap`]): while the key is undecided, the following
//! key events wait in a queue and are replayed in order once it's decided:
//! - pressed within [`Settings::require_prior_idle_ms`] after the press of a typed key: a tap at once;
//! - released before [`Settings::tapping_term_ms`]: a tap;
//! - another key pressed and released meanwhile ("permissive hold"): a hold;
//! - held for [`Settings::tapping_term_ms`]: a hold, so [`Keymap::tick`] must run while keys are held.
//!
//! Chords ([`ComboAction::Action`](crate::config::ComboAction::Action)): a press of a key of a chord waits in the queue too, until it's clear
//! whether the chord is pressed:
//! - every key of a chord pressed within [`Settings::combo_term_ms`] after the first one, nothing else pressed
//!   and no key of the chord released meanwhile: the presses are replaced with the chord action, held by
//!   the first key; if a longer chord starting with the same keys can still be pressed, it waits for it
//!   till the term ends. Keys pressed before the chord may be released meanwhile (rolling typing), those
//!   releases go before the chord action;
//! - otherwise the first key is pressed as usual and the following events are looked at again.
//!
//! The chord keys are matched before the tap-hold ones, so tap-hold keys can make a chord. The chord action
//! is released with the first key of the chord released, releasing the others then does nothing.
//!
//! Every change of a key reaches the host: [`Keymap::frame`] stops at a second change of the same key,
//! so a tap is a press in one report and a release in the next, and after a press, so keys replayed
//! from the queue are pressed in order even on hosts that sort the keys of one report.
//!
//! BLE profile keys ([`KeyAction::BtSelect`] etc.) never reach the report: a press queues a [`BtCommand`]
//! for the firmware, see [`Keymap::take_bt_command`].

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::config::{ComboKeyIdent, Config, JoystickButton, KeyAction, Layout, Settings};
use crate::keys::{Key, KeyMod};

/// Physical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyId {
    Matrix { x: u8, y: u8 },
    Joystick(JoystickButton),
}

impl KeyId {
    /// `(x, y)` of a matrix key.
    pub fn coord(self) -> Option<(u8, u8)> {
        match self {
            KeyId::Matrix { x, y } => Some((x, y)),
            KeyId::Joystick(_) => None,
        }
    }
}

/// What a BLE profile key asks the firmware to do, see [`Keymap::take_bt_command`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtCommand {
    /// Select profile `n`, [`KeyAction::BtSelect`].
    Select(u8),
    /// [`KeyAction::BtNext`].
    Next,
    /// [`KeyAction::BtPrev`].
    Prev,
    /// Forget the host of the current profile, [`KeyAction::BtClear`].
    Clear,
    /// Forget the hosts of all profiles, only a combo does it ([`crate::config::ComboAction::BtClearAll`]).
    ClearAll,
}

/// Key pressed or released, with the action it was pressed as (after [`KeyAction::Transparent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub key: KeyId,
    pub action: KeyAction,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy)]
struct Event {
    key: KeyId,
    pressed: bool,
    time: u32,
}

/// What a pressed key does until it's released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    None,
    /// Holds an action in the report.
    Report,
    /// Holds a layer in the stack.
    Layer(u8),
    /// Tap-hold key waiting for a decision, see [`Undecided`].
    Undecided,
    /// Key of a chord, the first key of the chord holds the chord action.
    Chord(KeyId),
}

#[derive(Debug, Clone, Copy)]
struct Held {
    key: KeyId,
    action: KeyAction,
    effect: Effect,
}

#[derive(Debug, Clone, Copy)]
enum Hold {
    Mods(KeyMod),
    Layer(u8),
}

#[derive(Debug, Clone, Copy)]
struct Undecided {
    key: KeyId,
    time: u32,
    tap: Key,
    hold: Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Tap,
    Hold,
}

/// Chord of the config, [`ComboAction::Action`](crate::config::ComboAction::Action).
#[derive(Debug, Clone)]
struct Chord {
    /// 2..=32 keys, at most one pressed key per ident.
    keys: Vec<ComboKeyIdent>,
    action: KeyAction,
}

/// What the presses at the front of the queue are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChordDecision {
    /// A chord may still be pressed.
    Wait,
    /// No chord: the first press is an ordinary one.
    None,
    /// The presses among the first `end` events are chord `chord`, the rest are releases of other keys.
    Fire { chord: usize, end: usize },
}

/// Change of the report: `Some` presses the action, `None` releases the key.
#[derive(Debug, Clone, Copy)]
struct Output {
    key: KeyId,
    action: Option<KeyAction>,
}

/// Runtime keymap of a [`Config`], see the module doc.
pub struct Keymap {
    layouts: Vec<Layout>,
    settings: Settings,
    chords: Vec<Chord>,
    base: u8,
    /// Active layers above `base`, the last one is on top.
    stack: Vec<u8>,
    /// Key events waiting for `undecided`.
    queue: VecDeque<Event>,
    undecided: Option<Undecided>,
    /// Processed pressed keys.
    held: Vec<Held>,
    /// Report changes not in [`Keymap::report`] yet.
    output: VecDeque<Output>,
    report: Vec<(KeyId, KeyAction)>,
    changes: Vec<Change>,
    /// Press time of the last typed key: a key, a key with modifiers or a tap.
    last_typed: Option<u32>,
    /// A toggle or a [`KeyAction::To`] changed the layers.
    layers_changed: bool,
    /// BLE profile commands of pressed keys, oldest first.
    bt_commands: VecDeque<BtCommand>,
}

impl Keymap {
    /// The first layout is the base layer. `config` should be valid: layer actions pointing to a missing
    /// layout do nothing, so [`Keymap::top_layer`] is always a layout.
    pub fn new(config: &Config) -> Self {
        Self {
            layouts: config.layouts.clone(),
            settings: config.settings,
            chords: config
                .combos
                .iter()
                // Fits the bit mask of `chord_match`.
                .filter(|combo| (2..=32).contains(&combo.keys.len()))
                .filter_map(|combo| {
                    let action = *combo.action.key_action()?;
                    Some(Chord { keys: combo.keys.iter().map(|key| key.ident).collect(), action })
                })
                .collect(),
            base: 0,
            stack: Vec::new(),
            queue: VecDeque::new(),
            undecided: None,
            held: Vec::new(),
            output: VecDeque::new(),
            report: Vec::new(),
            changes: Vec::new(),
            last_typed: None,
            layers_changed: false,
            bt_commands: VecDeque::new(),
        }
    }

    /// Panics if there is no such layout; the layers the keymap returns are always there.
    pub fn layout(&self, index: u8) -> &Layout {
        &self.layouts[index as usize]
    }

    /// Topmost active layer, what the keyboard shows as the current layout.
    pub fn top_layer(&self) -> u8 {
        self.stack.last().copied().unwrap_or(self.base)
    }

    /// Like [`KeyAction::To`]: `index` becomes the base layer, the others are turned off.
    /// `false` if there is no such layout.
    pub fn set_base(&mut self, index: u8) -> bool {
        if !self.has_layout(index) {
            return false;
        }
        self.base = index;
        self.stack.clear();
        true
    }

    /// Switches the base layer to the `permanent` layout of the [`KeyAction::Fn`] key in the base layer,
    /// if it has one. Returns the base layer.
    pub fn switch_layout(&mut self) -> u8 {
        let layout = &self.layouts[self.base as usize];
        let permanent = layout.fn_coord().and_then(|(x, y)| match layout.get(x, y) {
            Some(KeyAction::Fn { permanent, .. }) => Some(*permanent),
            _ => None,
        });
        if let Some(permanent) = permanent {
            self.set_base(permanent);
        }
        self.base
    }

    /// `true` once after a toggle or a [`KeyAction::To`] changed the layers, so the new
    /// [`Keymap::top_layer`] can be shown.
    pub fn take_layers_changed(&mut self) -> bool {
        core::mem::take(&mut self.layers_changed)
    }

    /// Next BLE profile command of a pressed key, in the order the keys were pressed.
    /// The firmware runs them after it sends the report, they aren't in [`Keymap::report`].
    pub fn take_bt_command(&mut self) -> Option<BtCommand> {
        self.bt_commands.pop_front()
    }

    /// `key` was pressed or released at `time` ms (wrapping).
    pub fn event(&mut self, key: KeyId, pressed: bool, time: u32) {
        self.queue.push_back(Event { key, pressed, time });
        self.run(time);
    }

    /// Decides tap-hold keys held past the tapping term, `now` is in ms (wrapping).
    pub fn tick(&mut self, now: u32) {
        self.run(now);
    }

    /// Applies the report changes for the next report: at most one per key and at most one press.
    pub fn frame(&mut self) {
        // Counted first, so no allocation per report; the queue is short.
        let mut count = 0;
        for (index, output) in self.output.iter().enumerate() {
            if self.output.iter().take(index).any(|earlier| earlier.key == output.key) {
                break;
            }
            count = index + 1;
            if output.action.is_some() {
                break;
            }
        }

        for output in self.output.drain(..count) {
            self.report.retain(|(key, _)| *key != output.key);
            if let Some(action) = output.action {
                self.report.push((output.key, action));
            }
        }
    }

    /// Actions held in the report, see [`Keymap::frame`]. Only basic actions, no layer or tap-hold ones.
    pub fn report(&self) -> impl Iterator<Item = &KeyAction> {
        self.report.iter().map(|(_, action)| action)
    }

    /// Keys pressed or released since the last call, in order.
    pub fn take_changes(&mut self) -> impl Iterator<Item = Change> + '_ {
        self.changes.drain(..)
    }

    /// Nothing waits: no undecided tap-hold key or chord, no queued event, no report change.
    pub fn is_idle(&self) -> bool {
        self.undecided.is_none() && self.queue.is_empty() && self.output.is_empty()
    }

    fn run(&mut self, now: u32) {
        loop {
            if let Some(undecided) = self.undecided {
                match self.decide(&undecided, now) {
                    Some(decision) => self.resolve(undecided, decision),
                    None => return,
                }
                continue;
            }

            let Some(&event) = self.queue.front() else {
                return;
            };
            if event.pressed && !self.chords.is_empty() && !self.is_held(event.key) {
                match self.decide_chord(now) {
                    ChordDecision::Wait => return,
                    ChordDecision::Fire { chord, end } => {
                        self.fire_chord(chord, end);
                        continue;
                    }
                    ChordDecision::None => {}
                }
            }

            self.queue.pop_front();
            if event.pressed {
                self.press(event);
            } else {
                self.release(event);
            }
        }
    }

    /// Looks at the queued events in order, then at the clock.
    fn decide(&self, undecided: &Undecided, now: u32) -> Option<Decision> {
        let term = self.settings.tapping_term_ms as u32;
        for (index, event) in self.queue.iter().enumerate() {
            if event.time.wrapping_sub(undecided.time) >= term {
                return Some(Decision::Hold);
            }
            if event.pressed {
                continue;
            }
            if event.key == undecided.key {
                return Some(Decision::Tap);
            }
            // Runs every tick while undecided, so it looks back instead of collecting the presses.
            let pressed_after = self.queue.iter().take(index).any(|earlier| earlier.pressed && earlier.key == event.key);
            if pressed_after {
                return Some(Decision::Hold);
            }
        }
        (now.wrapping_sub(undecided.time) >= term).then_some(Decision::Hold)
    }

    /// Looks at the presses at the front of the queue, then at the clock.
    fn decide_chord(&self, now: u32) -> ChordDecision {
        let term = self.settings.combo_term_ms as u32;
        let start = self.queue[0].time;
        // The longest complete chord so far, as (chord, end): it waits while a longer one may come.
        let mut complete = None;
        let mut ended = false;
        for (index, event) in self.queue.iter().enumerate() {
            if event.time.wrapping_sub(start) >= term {
                ended = true;
                break;
            }
            if !event.pressed {
                // A key pressed before the chord, released while typing on: it doesn't break the chord.
                if self.queue.iter().take(index).any(|earlier| earlier.pressed && earlier.key == event.key) {
                    ended = true;
                    break;
                }
                continue;
            }
            let end = index + 1;
            match self.chord_state(end) {
                (None, false) => {
                    ended = true;
                    break;
                }
                (Some(chord), false) => return ChordDecision::Fire { chord, end },
                (Some(chord), true) => complete = Some((chord, end)),
                (None, true) => {}
            }
        }
        if !ended && now.wrapping_sub(start) < term {
            return ChordDecision::Wait;
        }
        complete.map_or(ChordDecision::None, |(chord, end)| ChordDecision::Fire { chord, end })
    }

    /// For the presses among the first `end` queued events: a chord they make, whether a longer one may
    /// still be pressed.
    fn chord_state(&self, end: usize) -> (Option<usize>, bool) {
        let mut done = None;
        let mut pending = false;
        for (index, chord) in self.chords.iter().enumerate() {
            match self.chord_match(chord, end) {
                Some(true) => done = done.or(Some(index)),
                Some(false) => pending = true,
                None => {}
            }
        }
        (done, pending)
    }

    /// Whether the presses among the first `end` queued events are keys of `chord`, each one a different key
    /// of it: `Some(true)` if they are all of its keys.
    fn chord_match(&self, chord: &Chord, end: usize) -> Option<bool> {
        let mut used = 0u32;
        for event in self.queue.iter().take(end).filter(|event| event.pressed) {
            let (coord, action) = (event.key.coord(), self.lookup(event.key));
            let free = |index: usize, ident: &ComboKeyIdent, by_coord: bool| {
                used & (1 << index) == 0
                    && matches!(ident, ComboKeyIdent::Coord { .. }) == by_coord
                    && ident.matches(coord, &action)
            };
            // The position first: an action ident taken by a key that has its own position ident in the chord
            // could leave no ident for another key of that action.
            let index = [true, false]
                .into_iter()
                .find_map(|by_coord| chord.keys.iter().enumerate().position(|(index, ident)| free(index, ident, by_coord)))?;
            used |= 1 << index;
        }
        Some(used.count_ones() as usize == chord.keys.len())
    }

    /// Replaces the presses among the first `end` queued events with the action of `chord`, held by the first
    /// key; the releases among them happened before the chord was complete, so they go first.
    fn fire_chord(&mut self, chord: usize, end: usize) {
        let action = self.chords[chord].action;
        let events: Vec<Event> = self.queue.drain(..end).collect();
        for &event in events.iter().filter(|event| !event.pressed) {
            self.release(event);
        }

        let presses = || events.iter().filter(|event| event.pressed);
        let (Some(&first), Some(&last)) = (presses().next(), presses().next_back()) else {
            return;
        };
        for event in presses() {
            self.changes.push(Change { key: event.key, action, pressed: true });
            let effect = if event.key == first.key { Effect::None } else { Effect::Chord(first.key) };
            self.held.push(Held { key: event.key, action, effect });
        }
        // Pressed once the chord is complete.
        let effect = self.apply(Event { key: first.key, pressed: true, time: last.time }, action);
        if let Some(held) = self.held.iter_mut().find(|held| held.key == first.key) {
            held.effect = effect;
        }
    }

    fn resolve(&mut self, undecided: Undecided, decision: Decision) {
        self.undecided = None;
        let effect = match (decision, undecided.hold) {
            (Decision::Tap, _) => {
                self.last_typed = Some(undecided.time);
                self.output_press(undecided.key, KeyAction::Key(undecided.tap))
            }
            (Decision::Hold, Hold::Mods(mods)) => self.output_press(undecided.key, KeyAction::Mod(mods)),
            (Decision::Hold, Hold::Layer(layer)) => self.push_layer(layer),
        };
        if let Some(held) = self.held.iter_mut().find(|held| held.key == undecided.key) {
            held.effect = effect;
        }
    }

    fn press(&mut self, event: Event) {
        if self.is_held(event.key) {
            return;
        }
        let action = self.lookup(event.key);
        self.changes.push(Change { key: event.key, action, pressed: true });
        let effect = self.apply(event, action);
        self.held.push(Held { key: event.key, action, effect });
    }

    /// Starts what `action` of the pressed key does.
    fn apply(&mut self, event: Event, action: KeyAction) -> Effect {
        match action {
            KeyAction::Key(_) | KeyAction::KeyWithMod(..) => {
                self.last_typed = Some(event.time);
                self.output_press(event.key, action)
            }
            KeyAction::Mod(_)
            | KeyAction::Media(_)
            | KeyAction::MouseKey(_)
            | KeyAction::MouseTravel { .. }
            | KeyAction::MouseScroll { .. } => self.output_press(event.key, action),
            KeyAction::Fn { temporary: layer, .. } | KeyAction::Momentary(layer) => self.push_layer(layer),
            KeyAction::Toggle(layer) if self.has_layout(layer) => {
                match self.stack.iter().position(|&active| active == layer) {
                    Some(_) => self.stack.retain(|&active| active != layer),
                    None => self.stack.push(layer),
                }
                self.layers_changed = true;
                Effect::None
            }
            KeyAction::To(layer) => {
                self.layers_changed |= self.set_base(layer);
                Effect::None
            }
            KeyAction::ModTap { mods, tap } => self.tap_hold(event, tap, Hold::Mods(mods)),
            KeyAction::LayerTap { layer, tap } => self.tap_hold(event, tap, Hold::Layer(layer)),
            KeyAction::BtSelect(profile) => self.bt_command(BtCommand::Select(profile)),
            KeyAction::BtNext => self.bt_command(BtCommand::Next),
            KeyAction::BtPrev => self.bt_command(BtCommand::Prev),
            KeyAction::BtClear => self.bt_command(BtCommand::Clear),
            KeyAction::Toggle(_) | KeyAction::None | KeyAction::Transparent => Effect::None,
        }
    }

    fn release(&mut self, event: Event) {
        // Not pressed since the keymap was made.
        let Some(index) = self.held.iter().position(|held| held.key == event.key) else {
            return;
        };
        let held = self.held.remove(index);
        self.changes.push(Change { key: event.key, action: held.action, pressed: false });

        // The first key released of a chord releases the chord action, the others then do nothing.
        let (owner, effect) = match held.effect {
            Effect::Chord(owner) => {
                let effect = self.held.iter_mut().find(|held| held.key == owner).map(|held| &mut held.effect);
                (owner, effect.map_or(Effect::None, |effect| core::mem::replace(effect, Effect::None)))
            }
            effect => (event.key, effect),
        };
        for other in &mut self.held {
            if other.effect == Effect::Chord(owner) {
                other.effect = Effect::None;
            }
        }

        match effect {
            Effect::Report => self.output.push_back(Output { key: owner, action: None }),
            Effect::Layer(layer) => {
                if let Some(index) = self.stack.iter().rposition(|&active| active == layer) {
                    self.stack.remove(index);
                }
            }
            // `run` decides before it processes the release.
            Effect::None | Effect::Undecided | Effect::Chord(_) => {}
        }
    }

    fn tap_hold(&mut self, event: Event, tap: Key, hold: Hold) -> Effect {
        let idle = self.settings.require_prior_idle_ms as u32;
        let typing = self.last_typed.is_some_and(|typed| event.time.wrapping_sub(typed) < idle);
        if typing {
            self.last_typed = Some(event.time);
            return self.output_press(event.key, KeyAction::Key(tap));
        }
        self.undecided = Some(Undecided { key: event.key, time: event.time, tap, hold });
        Effect::Undecided
    }

    fn output_press(&mut self, key: KeyId, action: KeyAction) -> Effect {
        self.output.push_back(Output { key, action: Some(action) });
        Effect::Report
    }

    fn bt_command(&mut self, command: BtCommand) -> Effect {
        self.bt_commands.push_back(command);
        Effect::None
    }

    fn is_held(&self, key: KeyId) -> bool {
        self.held.iter().any(|held| held.key == key)
    }

    fn has_layout(&self, index: u8) -> bool {
        (index as usize) < self.layouts.len()
    }

    fn push_layer(&mut self, layer: u8) -> Effect {
        if !self.has_layout(layer) {
            return Effect::None;
        }
        self.stack.push(layer);
        Effect::Layer(layer)
    }

    /// Action of `key` in the topmost layer where it isn't [`KeyAction::Transparent`].
    fn lookup(&self, key: KeyId) -> KeyAction {
        let layers = self.stack.iter().rev().chain(core::iter::once(&self.base));
        for &layer in layers {
            let Some(layout) = self.layouts.get(layer as usize) else {
                continue;
            };
            let action = match key {
                KeyId::Matrix { x, y } => layout.get(x as usize, y as usize).copied().unwrap_or(KeyAction::None),
                KeyId::Joystick(button) => *layout.joystick.get(button),
            };
            if action != KeyAction::Transparent {
                return action;
            }
        }
        KeyAction::None
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;

    use super::*;
    use crate::config::{Combo, ComboAction, ComboKey, JoystickButtons, CONFIG_VERSION};

    const A: KeyId = KeyId::Matrix { x: 0, y: 0 };
    const B: KeyId = KeyId::Matrix { x: 1, y: 0 };
    const C: KeyId = KeyId::Matrix { x: 2, y: 0 };
    /// Ctrl when held, `F` when tapped.
    const CTRL_F: KeyId = KeyId::Matrix { x: 3, y: 0 };
    /// Layer 1 when held, `Space` when tapped.
    const LT1: KeyId = KeyId::Matrix { x: 4, y: 0 };
    const MO1: KeyId = KeyId::Matrix { x: 5, y: 0 };
    const TG2: KeyId = KeyId::Matrix { x: 6, y: 0 };
    const TO2: KeyId = KeyId::Matrix { x: 7, y: 0 };
    /// Shift when held, `J` when tapped.
    const SHIFT_J: KeyId = KeyId::Matrix { x: 8, y: 0 };
    /// BLE profile 2 in the base layer, the next profile in layer 1.
    const BT2: KeyId = KeyId::Matrix { x: 9, y: 0 };
    const STICK: KeyId = KeyId::Joystick(JoystickButton::Stick);

    const TERM: u32 = 200;
    const IDLE: u32 = 150;
    const COMBO_TERM: u32 = 50;

    fn key(key: Key) -> KeyAction {
        KeyAction::Key(key)
    }

    /// Layer 0: a b c Ctrl/F LT1/Space MO1 TG2 TO2 Shift/J BT2; layer 1: 1 . 3 ... BtNext (`.` transparent);
    /// layer 2: x . . and TO0 on the TO2 key; layer 2 joystick stick is transparent.
    fn keymap() -> Keymap {
        keymap_with(vec![])
    }

    /// [`keymap`] with chords: A+B is Esc, A+B+C is Tab, Ctrl/F+Shift/J is Enter, `1`+C is TG2.
    fn chord_keymap() -> Keymap {
        let coord = |key: KeyId| {
            let (x, y) = key.coord().unwrap();
            ComboKey { ident: ComboKeyIdent::Coord { x, y }, count: 0 }
        };
        let chord = |keys, action| Combo { keys, action: ComboAction::Action(action) };
        let one = ComboKey { ident: ComboKeyIdent::Action(key(Key::Num1)), count: 0 };
        keymap_with(vec![
            chord(vec![coord(A), coord(B)], key(Key::Escape)),
            chord(vec![coord(A), coord(B), coord(C)], key(Key::Tab)),
            chord(vec![coord(CTRL_F), coord(SHIFT_J)], key(Key::Enter)),
            chord(vec![one, coord(C)], KeyAction::Toggle(2)),
        ])
    }

    fn keymap_with(combos: Vec<Combo>) -> Keymap {
        let base = vec![
            key(Key::A),
            key(Key::B),
            key(Key::C),
            KeyAction::ModTap { mods: KeyMod::LeftCtrl, tap: Key::F },
            KeyAction::LayerTap { layer: 1, tap: Key::Space },
            KeyAction::Momentary(1),
            KeyAction::Toggle(2),
            KeyAction::To(2),
            KeyAction::ModTap { mods: KeyMod::LeftShift, tap: Key::J },
            KeyAction::BtSelect(2),
        ];
        let mut layer1 = vec![KeyAction::Transparent; base.len()];
        layer1[0] = key(Key::Num1);
        layer1[2] = key(Key::Num3);
        layer1[9] = KeyAction::BtNext;
        let mut layer2 = vec![KeyAction::Transparent; base.len()];
        layer2[0] = key(Key::X);
        layer2[7] = KeyAction::To(0);

        let layout = |name: &str, row, joystick| Layout { name: name.to_string(), rows: vec![row], joystick };
        let config = Config {
            version: CONFIG_VERSION,
            width: base.len() as u8,
            height: 1,
            layouts: vec![
                layout("BASE", base, JoystickButtons::default()),
                layout("ONE", layer1, JoystickButtons { stick: KeyAction::Momentary(2), ..JoystickButtons::default() }),
                layout("TWO", layer2, JoystickButtons {
                    stick: KeyAction::Transparent,
                    right: KeyAction::BtClear,
                    ..JoystickButtons::default()
                }),
            ],
            combos,
            settings: Settings {
                tapping_term_ms: TERM as u16,
                require_prior_idle_ms: IDLE as u16,
                combo_term_ms: COMBO_TERM as u16,
                ..Settings::default()
            },
        };
        config.validate().unwrap();
        Keymap::new(&config)
    }

    /// Feeds events and ticks, records every report.
    struct Run {
        keymap: Keymap,
        now: u32,
        reports: Vec<Vec<KeyAction>>,
    }

    impl Run {
        fn new() -> Self {
            Self { keymap: keymap(), now: 1000, reports: Vec::new() }
        }

        fn chords() -> Self {
            Self { keymap: chord_keymap(), now: 1000, reports: Vec::new() }
        }

        fn at(&mut self, now: u32) -> &mut Self {
            self.now = now;
            self.keymap.tick(now);
            self
        }

        fn down(&mut self, key: KeyId) -> &mut Self {
            self.keymap.event(key, true, self.now);
            self
        }

        fn up(&mut self, key: KeyId) -> &mut Self {
            self.keymap.event(key, false, self.now);
            self
        }

        /// One report, like the firmware loop sends.
        fn report(&mut self) -> &mut Self {
            self.keymap.tick(self.now);
            self.keymap.frame();
            self.reports.push(self.keymap.report().copied().collect());
            self
        }

        /// Reports until nothing waits, at most 10.
        fn flush(&mut self) -> &mut Self {
            for _ in 0..10 {
                self.report();
                if self.keymap.is_idle() {
                    break;
                }
            }
            self
        }

        /// The reports that differ from the previous one, without the empty first ones.
        fn sent(&self) -> Vec<Vec<KeyAction>> {
            let mut sent: Vec<Vec<KeyAction>> = Vec::new();
            for report in &self.reports {
                if sent.last().map_or(!report.is_empty(), |last| last != report) {
                    sent.push(report.clone());
                }
            }
            sent
        }
    }

    fn ctrl() -> KeyAction {
        KeyAction::Mod(KeyMod::LeftCtrl)
    }

    #[test]
    fn plain_key() {
        let mut run = Run::new();
        run.down(A).report().up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![]]);
    }

    #[test]
    fn tap() {
        let mut run = Run::new();
        run.down(CTRL_F).report();
        assert!(run.keymap.report().next().is_none(), "undecided, nothing sent");
        run.at(1100).up(CTRL_F).flush();
        assert_eq!(run.sent(), [vec![key(Key::F)], vec![]]);
        assert!(run.keymap.is_idle());
    }

    #[test]
    fn hold_by_timeout() {
        let mut run = Run::new();
        run.down(CTRL_F).report().at(1000 + TERM - 1).report();
        assert!(run.keymap.report().next().is_none());
        run.at(1000 + TERM).report();
        assert_eq!(run.sent(), [vec![ctrl()]]);
        run.at(1500).down(A).report().up(A).report().up(CTRL_F).flush();
        assert_eq!(run.sent(), [vec![ctrl()], vec![ctrl(), key(Key::A)], vec![ctrl()], vec![]]);
    }

    #[test]
    fn permissive_hold() {
        let mut run = Run::new();
        // Ctrl+A typed well within the tapping term: A pressed and released while Ctrl/F is held.
        run.down(CTRL_F).report().at(1050).down(A).report().at(1080).up(A).report().at(1100).up(CTRL_F).flush();
        assert_eq!(run.sent(), [vec![ctrl()], vec![ctrl(), key(Key::A)], vec![]]);
    }

    #[test]
    fn rolling_is_a_tap() {
        let mut run = Run::new();
        // "fa" typed fast: A goes down before F is up, but is released after it.
        run.down(CTRL_F).report().at(1050).down(A).report().at(1080).up(CTRL_F).report().at(1100).up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::F)], vec![key(Key::F), key(Key::A)], vec![]]);
    }

    #[test]
    fn key_held_before_is_not_an_interruption() {
        let mut run = Run::new();
        // B down, Ctrl/F down, B up, F up: B's release doesn't make F a hold.
        run.down(B).report().at(1300).down(CTRL_F).report().at(1320).up(B).report().at(1340).up(CTRL_F).flush();
        assert_eq!(run.sent(), [vec![key(Key::B)], vec![key(Key::B), key(Key::F)], vec![]]);
    }

    #[test]
    fn prior_idle_makes_a_tap_at_once() {
        let mut run = Run::new();
        run.down(A).report().at(1050).up(A).report();
        // Pressed 100 ms after A: a tap at once, even if held past the tapping term and interrupted.
        run.at(1100).down(CTRL_F).report().at(1120).down(B).report().up(B).report().at(1400).up(CTRL_F).flush();
        assert_eq!(
            run.sent(),
            [vec![key(Key::A)], vec![], vec![key(Key::F)], vec![key(Key::F), key(Key::B)], vec![key(Key::F)], vec![]]
        );

        // After the idle time it's a tap-hold again.
        let mut run = Run::new();
        run.down(A).report().up(A).report().at(1000 + IDLE).down(CTRL_F).report().at(1000 + IDLE + TERM).report();
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![], vec![ctrl()]]);
    }

    #[test]
    fn held_modifier_doesnt_count_as_typing() {
        let mut run = Run::new();
        run.at(700).down(A).up(A).flush();
        // Ctrl from a held Ctrl/F, then Shift/J right after it: still a tap-hold, both are held.
        run.at(1000).down(CTRL_F).at(1000 + TERM).report().at(1210).down(SHIFT_J).at(1210 + TERM).report();
        let shift = KeyAction::Mod(KeyMod::LeftShift);
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![], vec![ctrl()], vec![ctrl(), shift]]);
    }

    #[test]
    fn events_wait_and_keep_their_order() {
        let mut run = Run::new();
        // B, C pressed and released after the tap-hold key, all before it's decided.
        run.down(LT1).at(1010).down(B).at(1020).up(B).at(1030).down(C).at(1040).up(C).at(1050).up(LT1).flush();
        // B pressed and released: LT1 is a hold, so C is looked up in layer 1; B is transparent there.
        assert_eq!(run.sent(), [vec![key(Key::B)], vec![key(Key::Num3)], vec![]]);
    }

    #[test]
    fn nested_tap_holds() {
        let mut run = Run::new();
        // Ctrl/F held, Shift/J tapped inside it: Ctrl/F is a hold, Shift/J a tap.
        run.down(CTRL_F).at(1020).down(SHIFT_J).at(1040).up(SHIFT_J).report().at(1060).up(CTRL_F).flush();
        assert_eq!(run.sent(), [vec![ctrl()], vec![ctrl(), key(Key::J)], vec![]]);
    }

    #[test]
    fn layer_tap_hold_switches_layer() {
        let mut run = Run::new();
        run.down(LT1).at(1000 + TERM).report().down(A).report().up(A).report().up(LT1).report();
        run.down(A).report().up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::Num1)], vec![], vec![key(Key::A)], vec![]]);
    }

    #[test]
    fn transparent_falls_through() {
        let mut run = Run::new();
        // Layer 1: A is 1, B is transparent, so it's B of the base layer.
        run.down(MO1).down(A).down(B).report().report();
        assert_eq!(run.sent(), [vec![key(Key::Num1)], vec![key(Key::Num1), key(Key::B)]]);
        // Layer 2's stick is transparent: the one of layer 1 below it, or of the base layer.
        run.down(TG2);
        assert_eq!(run.keymap.top_layer(), 2);
        assert_eq!(run.keymap.lookup(STICK), KeyAction::Momentary(2));
        run.up(TG2).up(MO1);
        assert_eq!(run.keymap.top_layer(), 2);
        assert_eq!(run.keymap.lookup(STICK), *run.keymap.layout(0).joystick.get(JoystickButton::Stick));

        // Transparent in the base layer does nothing.
        let mut keymap = keymap();
        keymap.layouts[0].rows[0][1] = KeyAction::Transparent;
        assert_eq!(keymap.lookup(B), KeyAction::None);
    }

    #[test]
    fn momentary_release_keeps_what_was_pressed() {
        let mut run = Run::new();
        // A pressed in layer 1 sends 1 until released, even if the layer is gone.
        run.down(MO1).down(A).report().up(MO1).report();
        assert_eq!(run.keymap.top_layer(), 0);
        run.up(A).report().down(A).report();
        assert_eq!(run.sent(), [vec![key(Key::Num1)], vec![], vec![key(Key::A)]]);
    }

    #[test]
    fn toggle_and_momentary_stack() {
        let mut run = Run::new();
        run.down(TG2).up(TG2);
        assert!(run.keymap.take_layers_changed());
        assert_eq!(run.keymap.top_layer(), 2);
        // MO1 on top of the toggled layer 2, then gone again.
        run.down(MO1);
        assert_eq!(run.keymap.top_layer(), 1);
        assert_eq!(run.keymap.lookup(A), key(Key::Num1));
        run.up(MO1);
        assert_eq!(run.keymap.top_layer(), 2);
        assert_eq!(run.keymap.lookup(A), key(Key::X));
        // B is transparent in 2: the base one.
        assert_eq!(run.keymap.lookup(B), key(Key::B));
        // TG2 is transparent in layer 2 too, so it toggles 2 off.
        run.down(TG2).up(TG2);
        assert_eq!(run.keymap.top_layer(), 0);
        assert!(run.keymap.take_layers_changed());
        assert!(!run.keymap.take_layers_changed());
    }

    #[test]
    fn to_replaces_base_and_clears_the_stack() {
        let mut run = Run::new();
        run.down(MO1).down(TO2).up(TO2);
        assert_eq!(run.keymap.top_layer(), 2);
        assert!(run.keymap.take_layers_changed());
        // Releasing MO1 after TO cleared it changes nothing.
        run.up(MO1);
        assert_eq!(run.keymap.top_layer(), 2);
        // Layer 2 has TO0 on the same key.
        run.down(TO2).up(TO2);
        assert_eq!(run.keymap.top_layer(), 0);
    }

    #[test]
    fn joystick_button_uses_layers() {
        let mut run = Run::new();
        // MO2 on the stick in layer 1: hold MO1, then the stick.
        run.down(MO1).down(STICK);
        assert_eq!(run.keymap.top_layer(), 2);
        run.up(STICK).up(MO1);
        assert_eq!(run.keymap.top_layer(), 0);
    }

    #[test]
    fn fn_key_and_switch_layout() {
        let mut keymap = keymap();
        keymap.layouts[0].rows[0][5] = KeyAction::Fn { temporary: 1, permanent: 2 };
        let mut run = Run { keymap, now: 0, reports: Vec::new() };
        run.down(MO1);
        assert_eq!(run.keymap.top_layer(), 1);
        run.up(MO1);
        assert_eq!(run.keymap.switch_layout(), 2);
        assert_eq!(run.keymap.top_layer(), 2);
        assert!(run.keymap.set_base(0));
        assert!(!run.keymap.set_base(3));
    }

    #[test]
    fn changes_have_the_pressed_action() {
        let mut run = Run::new();
        run.down(MO1).down(A).up(A);
        let changes: Vec<Change> = run.keymap.take_changes().collect();
        assert_eq!(
            changes,
            [
                Change { key: MO1, action: KeyAction::Momentary(1), pressed: true },
                Change { key: A, action: key(Key::Num1), pressed: true },
                Change { key: A, action: key(Key::Num1), pressed: false },
            ]
        );
        assert_eq!(run.keymap.take_changes().count(), 0);
    }

    #[test]
    fn bt_keys_are_commands_not_reports() {
        let mut run = Run::new();
        // A held, BT2 tapped: only A reaches the host, the command waits for the firmware.
        run.down(A).report().down(BT2).report().up(BT2).report().up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![]]);
        assert_eq!(run.keymap.take_bt_command(), Some(BtCommand::Select(2)));
        assert_eq!(run.keymap.take_bt_command(), None);
        assert!(run.keymap.is_idle());

        // The same key in layer 1 is the next profile; commands keep the order of the presses.
        run.down(MO1).down(BT2).up(BT2).up(MO1).down(BT2).up(BT2);
        assert_eq!(run.keymap.take_bt_command(), Some(BtCommand::Next));
        assert_eq!(run.keymap.take_bt_command(), Some(BtCommand::Select(2)));
        assert_eq!(run.keymap.take_bt_command(), None);

        // A joystick button too, and combos see the press like any other.
        run.down(TG2).up(TG2).down(KeyId::Joystick(JoystickButton::Right));
        assert_eq!(run.keymap.take_bt_command(), Some(BtCommand::Clear));
        let last = run.keymap.take_changes().last();
        assert_eq!(
            last,
            Some(Change { key: KeyId::Joystick(JoystickButton::Right), action: KeyAction::BtClear, pressed: true })
        );
    }

    #[test]
    fn bt_key_after_a_tap_hold_waits_for_the_decision() {
        let mut run = Run::new();
        // LT1 held, BT2 pressed and released meanwhile: a hold, so BT2 is the next profile of layer 1.
        run.down(LT1).at(1010).down(BT2).at(1020).up(BT2);
        assert_eq!(run.keymap.take_bt_command(), Some(BtCommand::Next));
        run.at(1030).up(LT1).flush();
        assert!(run.sent().is_empty());
    }

    #[test]
    fn time_wraps() {
        let mut run = Run::new();
        run.at(u32::MAX - 50).down(CTRL_F).report().at(100).report();
        assert!(run.sent().is_empty());
        run.at(200).report();
        assert_eq!(run.sent(), [vec![ctrl()]]);
    }

    #[test]
    fn missing_layers_do_nothing() {
        let mut run = Run::new();
        // An unvalidated config: the layer keys point past the 3 layouts.
        run.keymap.layouts[0].rows[0][5] = KeyAction::Momentary(7);
        run.keymap.layouts[0].rows[0][6] = KeyAction::Toggle(7);
        run.keymap.layouts[0].rows[0][7] = KeyAction::To(7);
        run.down(MO1).down(TG2).down(TO2).report();
        assert_eq!(run.keymap.top_layer(), 0);
        assert!(!run.keymap.take_layers_changed());
        run.down(A).up(A).up(MO1).up(TG2).up(TO2).flush();
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![]]);
    }

    #[test]
    fn chord_waits_for_a_longer_one() {
        let mut run = Run::chords();
        run.down(A).at(1010).down(B).report();
        assert!(run.keymap.report().next().is_none(), "A+B+C may still come");
        assert!(!run.keymap.is_idle());
        run.at(1000 + COMBO_TERM).report();
        assert_eq!(run.sent(), [vec![key(Key::Escape)]]);
        // The first key released releases the chord, the other one does nothing.
        run.at(1100).up(B).report().at(1120).up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::Escape)], vec![]]);
        assert!(run.keymap.is_idle());
        let changes: Vec<Change> = run.keymap.take_changes().collect();
        let escape = key(Key::Escape);
        assert_eq!(
            changes,
            [
                Change { key: A, action: escape, pressed: true },
                Change { key: B, action: escape, pressed: true },
                Change { key: B, action: escape, pressed: false },
                Change { key: A, action: escape, pressed: false },
            ]
        );
    }

    #[test]
    fn longest_chord_fires_at_once() {
        let mut run = Run::chords();
        run.down(C).at(1010).down(A).at(1020).down(B).report();
        assert_eq!(run.sent(), [vec![key(Key::Tab)]]);
        run.at(1100).up(A).report();
        // Pressed again alone, A is A again.
        run.at(1300).down(A).at(1300 + COMBO_TERM).report().up(A).up(B).up(C).flush();
        assert_eq!(run.sent(), [vec![key(Key::Tab)], vec![], vec![key(Key::A)], vec![]]);
    }

    #[test]
    fn slow_chord_keys_are_ordinary() {
        let mut run = Run::chords();
        run.down(A).at(1000 + COMBO_TERM).down(B).at(1000 + 2 * COMBO_TERM).report().up(A).up(B).flush();
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![key(Key::A), key(Key::B)], vec![]]);
    }

    #[test]
    fn other_key_or_release_breaks_a_chord() {
        // A key of no chord: A is pressed at once, as what it was before MO1.
        let mut run = Run::chords();
        run.down(A).at(1010).down(MO1).report();
        assert_eq!(run.sent(), [vec![key(Key::A)]]);
        assert_eq!(run.keymap.top_layer(), 1);

        // A tapped fast is A.
        let mut run = Run::chords();
        run.down(A).at(1010).up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::A)], vec![]]);

        // A, B, then A released before C: the A+B chord.
        let mut run = Run::chords();
        run.down(A).at(1010).down(B).at(1020).up(A).flush();
        assert_eq!(run.sent(), [vec![key(Key::Escape)], vec![]]);
    }

    #[test]
    fn release_of_an_earlier_key_doesnt_break_a_chord() {
        let mut run = Run::chords();
        // Rolling typing: the stick (middle button) is released between A and B.
        run.down(STICK).report().at(1100).down(A).at(1110).up(STICK).at(1120).down(B).at(1100 + COMBO_TERM).flush();
        let middle = KeyAction::MouseKey(crate::keys::MouseKey::Middle);
        // The release and the chord press are different keys, they go in one report.
        assert_eq!(run.sent(), [vec![middle], vec![key(Key::Escape)]]);
        let changes: Vec<Change> = run.keymap.take_changes().collect();
        assert_eq!(changes[1], Change { key: STICK, action: middle, pressed: false }, "the release goes first");
        assert_eq!(changes[2], Change { key: A, action: key(Key::Escape), pressed: true });
    }

    #[test]
    fn tap_hold_keys_make_a_chord() {
        let mut run = Run::chords();
        run.down(SHIFT_J).at(1020).down(CTRL_F).report().at(1000 + TERM + 10).report();
        assert_eq!(run.sent(), [vec![key(Key::Enter)]]);
        run.up(CTRL_F).up(SHIFT_J).flush();
        assert_eq!(run.sent(), [vec![key(Key::Enter)], vec![]]);

        // Alone it's a tap-hold key again.
        run.at(2000).down(CTRL_F).at(2000 + TERM).report();
        assert_eq!(run.sent(), [vec![key(Key::Enter)], vec![], vec![ctrl()]]);
    }

    #[test]
    fn chord_key_by_position_leaves_the_action_ident_to_another_key() {
        // A and B both type `a`, the chord is any `a` key and the A position: A pressed first is the position.
        let mut keymap = chord_keymap();
        keymap.layouts[0].rows[0][1] = key(Key::A);
        let keys = vec![ComboKeyIdent::Action(key(Key::A)), ComboKeyIdent::Coord { x: 0, y: 0 }];
        keymap.chords = vec![Chord { keys, action: key(Key::Escape) }];
        let mut run = Run { keymap, now: 1000, reports: Vec::new() };
        run.down(A).at(1010).down(B).report();
        assert_eq!(run.sent(), [vec![key(Key::Escape)]]);
    }

    #[test]
    fn chord_key_matched_by_action_in_the_current_layer() {
        let mut run = Run::chords();
        // A is `1` in layer 1: `1`+C toggles layer 2, once A+B+C can't come any more.
        run.down(MO1).down(A).at(1010).down(C);
        assert!(!run.keymap.take_layers_changed());
        run.at(1000 + COMBO_TERM);
        assert!(run.keymap.take_layers_changed());
        run.up(A).up(C).up(MO1).flush();
        assert_eq!(run.keymap.top_layer(), 2);
        assert!(run.sent().is_empty());
    }
}
