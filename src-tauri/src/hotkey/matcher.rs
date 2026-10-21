//! Ordered hotkey state machine. Time is supplied by the listener for deterministic tests.
use keytap::{EventKind, Key};
use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::{HotkeyBinding, HotkeyMode};

pub const HOLD_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyAction {
    StartRecording { is_main: bool, mode: HotkeyMode },
    StopRecording { prompt_id: Option<String> },
}

#[derive(Debug, Default)]
pub struct MatcherState {
    pub(super) held: HashSet<Key>,
    // Union of keys in this cycle: removing an extra key never restores a candidate.
    cycle_keys: HashSet<Key>,
    candidate: Option<HotkeyBinding>,
    pending: Option<(HotkeyBinding, Instant)>,
    active: Option<HotkeyBinding>,
    blocked: bool,
    generation: u64,
    active_stops: Vec<HotkeyBinding>,
}

impl MatcherState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn recording_mode(&self) -> Option<HotkeyMode> {
        self.active.as_ref().map(|b| b.mode)
    }

    /// Preserve physical state across cancel, retry and failed start. Require release
    /// before another trigger, rather than treating a still-held key as a new press.
    pub fn reset_recording(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.active_stops.clear();
        self.active = None;
        self.cancel_pending();
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
        self.candidate = None;
        self.blocked = !self.held.is_empty();
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pending_deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, at)| *at + HOLD_DELAY)
    }

    pub fn tick(&mut self, now: Instant) -> Vec<HotkeyAction> {
        let Some((binding, since)) = self.pending.as_ref() else {
            return vec![];
        };
        if now.saturating_duration_since(*since) < HOLD_DELAY {
            return vec![];
        }
        let binding = binding.clone();
        self.pending = None;
        if self.blocked || !exact(&self.held, &binding) {
            return vec![];
        }
        self.active = Some(binding);
        vec![HotkeyAction::StartRecording {
            is_main: false,
            mode: HotkeyMode::Hold,
        }]
    }

    pub fn process_at(
        &mut self,
        kind: EventKind,
        bindings: &[HotkeyBinding],
        now: Instant,
    ) -> Vec<HotkeyAction> {
        // A queued event can arrive after the deadline before the timeout tick.
        let mut actions = self.tick(now);
        let stop_bindings = self.active_stops.clone();
        let bindings = if self.recording_mode() == Some(HotkeyMode::Toggle) {
            &stop_bindings[..]
        } else {
            bindings
        };
        match kind {
            EventKind::KeyRepeat(_) => return actions,
            EventKind::KeyDown(key) => {
                if !self.held.insert(key) {
                    return actions;
                }
                if matches!(key, Key::CapsLock | Key::NumLock | Key::ScrollLock)
                    && !bindings.iter().any(|b| b.keys.contains(&key))
                {
                    self.held.remove(&key);
                    if self.recording_mode() != Some(HotkeyMode::Hold) {
                        self.blocked = !self.held.is_empty();
                        self.pending = None;
                        self.candidate = None;
                    }
                    return actions;
                }
                // Lock hold sessions to the original combination and template.
                if self.recording_mode() == Some(HotkeyMode::Hold) {
                    return actions;
                }
                self.cycle_keys.insert(key);
                if self.blocked {
                    return actions;
                }
                let is_recording = self.active.is_some();
                // Every key observed in the cycle must belong to a possible full
                // binding; a foreign key permanently invalidates this cycle.
                let eligible = |b: &&HotkeyBinding| {
                    if is_recording {
                        b.mode == HotkeyMode::Toggle && !b.wake_only
                    } else {
                        b.wake_only || b.mode == HotkeyMode::Hold
                    }
                };
                if !bindings
                    .iter()
                    .filter(eligible)
                    .any(|b| self.cycle_keys.iter().all(|k| b.keys.contains(k)))
                {
                    self.blocked = true;
                    self.pending = None;
                    self.candidate = None;
                    return actions;
                }
                let matched = bindings
                    .iter()
                    .filter(eligible)
                    .find(|b| exact(&self.held, b) && exact(&self.cycle_keys, b));
                self.candidate = matched.cloned();
                self.pending = matched
                    .filter(|b| b.mode == HotkeyMode::Hold)
                    .map(|b| (b.clone(), now));
            }
            EventKind::KeyUp(key) => {
                let was_held = self.held.remove(&key);
                if !was_held {
                    return actions;
                }
                if let Some(active) = self.active.as_ref().filter(|b| b.mode == HotkeyMode::Hold) {
                    if active.keys.contains(&key) {
                        let prompt_id = active.prompt_id.clone();
                        self.active = None;
                        self.cancel_pending();
                        actions.push(HotkeyAction::StopRecording { prompt_id });
                    }
                } else {
                    self.pending = None;
                    if self.held.is_empty() && !self.blocked {
                        if let Some(binding) = self
                            .candidate
                            .take()
                            .filter(|b| b.mode == HotkeyMode::Toggle && exact(&self.cycle_keys, b))
                        {
                            if self.active.is_some() {
                                self.active = None;
                                self.active_stops.clear();
                                actions.push(HotkeyAction::StopRecording {
                                    prompt_id: binding.prompt_id,
                                });
                            } else {
                                self.active = Some(binding);
                                self.active_stops = bindings
                                    .iter()
                                    .filter(|b| !b.wake_only && b.mode == HotkeyMode::Toggle)
                                    .cloned()
                                    .collect();
                                actions.push(HotkeyAction::StartRecording {
                                    is_main: true,
                                    mode: HotkeyMode::Toggle,
                                });
                            }
                        }
                    }
                }
                if self.held.is_empty() {
                    self.cycle_keys.clear();
                    self.candidate = None;
                    self.pending = None;
                    self.blocked = false;
                }
            }
        }
        actions
    }

    /// Lost-key recovery never submits output: only known released keys are removed.
    /// Missing releases invalidate the cycle; an active hold is cancelled by caller.
    pub fn recover_released(&mut self, released: &[Key]) -> bool {
        let cancel_hold = self.active.as_ref().is_some_and(|b| {
            b.mode == HotkeyMode::Hold && released.iter().any(|k| b.keys.contains(k))
        });
        for key in released {
            self.held.remove(key);
        }
        if !released.is_empty() {
            self.cancel_pending();
        }
        if self.held.is_empty() {
            self.cycle_keys.clear();
            self.blocked = false;
        }
        cancel_hold
    }

    /// Only toggle commands swallow characters. Hold candidates and active holds
    /// leave ordinary application shortcuts alone throughout their lifetime.
    pub(super) fn should_swallow(
        &self,
        key: Key,
        bindings: &[HotkeyBinding],
        modifier_states: &[(Key, bool)],
    ) -> bool {
        if is_modifier(key) || self.blocked || self.recording_mode() == Some(HotkeyMode::Hold) {
            return false;
        }
        let mut held = self.held.clone();
        for (modifier, pressed) in modifier_states {
            if *pressed {
                held.insert(*modifier);
            } else {
                held.remove(modifier);
            }
        }
        bindings.iter().any(|b| {
            b.mode == HotkeyMode::Toggle
                && (if self.active.is_some() {
                    !b.wake_only
                } else {
                    b.wake_only
                })
                && b.keys.contains(&key)
                && b.keys.iter().all(|k| *k == key || held.contains(k))
                && held
                    .iter()
                    .chain(self.cycle_keys.iter())
                    .all(|k| b.keys.contains(k))
        })
    }
}

fn exact(keys: &HashSet<Key>, binding: &HotkeyBinding) -> bool {
    keys.len() == binding.keys.len() && keys.iter().all(|k| binding.keys.contains(k))
}

pub(super) fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ControlLeft
            | Key::ControlRight
            | Key::ShiftLeft
            | Key::ShiftRight
            | Key::AltLeft
            | Key::AltRight
            | Key::MetaLeft
            | Key::MetaRight
            | Key::Function
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventKind::{KeyDown as down, KeyUp as up};

    fn binding(
        keys: &[Key],
        mode: HotkeyMode,
        prompt: Option<&str>,
        wake_only: bool,
    ) -> HotkeyBinding {
        HotkeyBinding {
            keys: keys.iter().copied().collect(),
            mode,
            prompt_id: prompt.map(str::to_string),
            wake_only,
        }
    }
    fn hold(keys: &[Key]) -> Vec<HotkeyBinding> {
        vec![binding(keys, HotkeyMode::Hold, Some("polish"), false)]
    }
    fn toggle() -> Vec<HotkeyBinding> {
        vec![
            binding(&[Key::MetaRight], HotkeyMode::Toggle, None, true),
            binding(&[Key::MetaRight], HotkeyMode::Toggle, None, false),
        ]
    }
    fn event(
        m: &mut MatcherState,
        b: &[HotkeyBinding],
        base: Instant,
        ms: u64,
        e: EventKind,
    ) -> Vec<HotkeyAction> {
        m.process_at(e, b, base + Duration::from_millis(ms))
    }
    fn start(mode: HotkeyMode) -> HotkeyAction {
        HotkeyAction::StartRecording {
            is_main: mode == HotkeyMode::Toggle,
            mode,
        }
    }

    #[test]
    fn left_modifier_starts_and_shift_finishes_with_general_polish() {
        for main in [Key::MetaLeft, Key::ControlLeft] {
            let b = vec![
                binding(&[main], HotkeyMode::Toggle, None, true),
                binding(&[main], HotkeyMode::Toggle, None, false),
                binding(
                    &[Key::ShiftLeft],
                    HotkeyMode::Toggle,
                    Some("prompt-general"),
                    false,
                ),
            ];
            let mut m = MatcherState::new();
            let t = Instant::now();
            // Shift is a stop key only; ordinary modifier chords remain usable.
            for (ms, e) in [
                down(Key::ShiftLeft),
                up(Key::ShiftLeft),
                down(main),
                down(Key::C),
                up(Key::C),
                up(main),
            ]
            .into_iter()
            .enumerate()
            {
                assert!(event(&mut m, &b, t, ms as u64, e).is_empty());
            }
            assert!(event(&mut m, &b, t, 10, down(main)).is_empty());
            assert_eq!(
                event(&mut m, &b, t, 11, up(main)),
                vec![start(HotkeyMode::Toggle)]
            );
            for (ms, e) in [
                down(Key::ShiftLeft),
                down(Key::A),
                up(Key::A),
                up(Key::ShiftLeft),
            ]
            .into_iter()
            .enumerate()
            {
                assert!(event(&mut m, &b, t, 20 + ms as u64, e).is_empty());
            }
            assert!(event(&mut m, &b, t, 30, down(Key::ShiftLeft)).is_empty());
            assert_eq!(
                event(&mut m, &b, t, 31, up(Key::ShiftLeft)),
                vec![HotkeyAction::StopRecording {
                    prompt_id: Some("prompt-general".into())
                }]
            );
            event(&mut m, &b, t, 40, down(main));
            assert_eq!(
                event(&mut m, &b, t, 41, up(main)),
                vec![start(HotkeyMode::Toggle)]
            );
            event(&mut m, &b, t, 50, down(main));
            assert_eq!(
                event(&mut m, &b, t, 51, up(main)),
                vec![HotkeyAction::StopRecording { prompt_id: None }]
            );
        }
    }

    #[test]
    fn hold_waits_for_full_500ms() {
        let b = hold(&[Key::ControlLeft]);
        let mut m = MatcherState::new();
        let t = Instant::now();
        assert!(event(&mut m, &b, t, 0, down(Key::ControlLeft)).is_empty());
        assert!(m.tick(t + Duration::from_millis(499)).is_empty());
        assert_eq!(m.tick(t + HOLD_DELAY), vec![start(HotkeyMode::Hold)]);
        assert!(m.tick(t + HOLD_DELAY).is_empty());
        assert_eq!(
            event(&mut m, &b, t, 501, up(Key::ControlLeft)),
            vec![HotkeyAction::StopRecording {
                prompt_id: Some("polish".into())
            }]
        );
    }
    #[test]
    fn extra_key_at_200ms_invalidates_whole_hold_cycle() {
        let b = hold(&[Key::ControlLeft]);
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::ControlLeft));
        event(&mut m, &b, t, 200, down(Key::C));
        event(&mut m, &b, t, 220, up(Key::C));
        assert!(m.tick(t + Duration::from_secs(1)).is_empty());
        event(&mut m, &b, t, 1100, up(Key::ControlLeft));
        event(&mut m, &b, t, 1200, down(Key::ControlLeft));
        assert_eq!(
            m.tick(t + Duration::from_millis(1700)),
            vec![start(HotkeyMode::Hold)]
        );
    }
    #[test]
    fn combo_delay_begins_at_last_key_regardless_of_prefix_duration() {
        for prefix_ms in [200, 600, 3000] {
            let b = hold(&[Key::ControlLeft, Key::C]);
            let mut m = MatcherState::new();
            let t = Instant::now();
            event(&mut m, &b, t, 0, down(Key::ControlLeft));
            assert!(m.tick(t + Duration::from_millis(prefix_ms)).is_empty());
            event(&mut m, &b, t, prefix_ms, down(Key::C));
            assert!(m
                .tick(t + Duration::from_millis(prefix_ms + 499))
                .is_empty());
            assert_eq!(
                m.tick(t + Duration::from_millis(prefix_ms + 500)),
                vec![start(HotkeyMode::Hold)]
            );
        }
    }
    #[test]
    fn release_before_deadline_never_records() {
        let b = hold(&[Key::ControlLeft]);
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::ControlLeft));
        assert!(event(&mut m, &b, t, 499, up(Key::ControlLeft)).is_empty());
        assert!(m.tick(t + Duration::from_secs(1)).is_empty());
    }
    #[test]
    fn release_at_deadline_orders_start_before_stop_even_without_tick() {
        let b = hold(&[Key::ControlLeft]);
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::ControlLeft));
        assert_eq!(
            event(&mut m, &b, t, 500, up(Key::ControlLeft)),
            vec![
                start(HotkeyMode::Hold),
                HotkeyAction::StopRecording {
                    prompt_id: Some("polish".into())
                }
            ]
        );
    }
    #[test]
    fn active_hold_ignores_new_keys_templates_and_new_bindings() {
        let mut b = hold(&[Key::ControlLeft]);
        b.push(binding(&[Key::C], HotkeyMode::Toggle, Some("other"), false));
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::ControlLeft));
        assert_eq!(
            event(&mut m, &b, t, 600, down(Key::C)),
            vec![start(HotkeyMode::Hold)]
        );
        assert!(!m.should_swallow(Key::C, &b, &[]));
        assert_eq!(
            event(&mut m, &[], t, 700, up(Key::ControlLeft)),
            vec![HotkeyAction::StopRecording {
                prompt_id: Some("polish".into())
            }]
        );
        assert!(event(&mut m, &b, t, 710, up(Key::C)).is_empty());
    }
    #[test]
    fn both_extra_key_orders_block_toggle_and_no_release_fallback() {
        for extra in [Key::F, Key::ShiftLeft, Key::Function] {
            for first in [true, false] {
                let b = toggle();
                let mut m = MatcherState::new();
                let t = Instant::now();
                let (a, c) = if first {
                    (Key::MetaRight, extra)
                } else {
                    (extra, Key::MetaRight)
                };
                for (ms, e) in [down(a), down(c), up(c), up(a)].into_iter().enumerate() {
                    assert!(event(&mut m, &b, t, ms as u64, e).is_empty(), "{a:?} {c:?}");
                }
            }
        }
    }
    #[test]
    fn longer_toggle_stop_wins_without_firing_short_stop() {
        let mut b = toggle();
        b.push(binding(
            &[Key::MetaRight, Key::ShiftLeft],
            HotkeyMode::Toggle,
            Some("long"),
            false,
        ));
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::MetaRight));
        assert_eq!(
            event(&mut m, &b, t, 1, up(Key::MetaRight)),
            vec![start(HotkeyMode::Toggle)]
        );
        event(&mut m, &b, t, 2, down(Key::MetaRight));
        event(&mut m, &b, t, 3, down(Key::ShiftLeft));
        assert!(event(&mut m, &b, t, 4, up(Key::ShiftLeft)).is_empty());
        assert_eq!(
            event(&mut m, &b, t, 5, up(Key::MetaRight)),
            vec![HotkeyAction::StopRecording {
                prompt_id: Some("long".into())
            }]
        );
    }
    #[test]
    fn raw_custom_stop_and_global_wake_are_independent() {
        let b = vec![
            binding(&[Key::MetaRight], HotkeyMode::Toggle, None, true),
            binding(&[Key::F2], HotkeyMode::Toggle, None, false),
        ];
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::MetaRight));
        assert_eq!(
            event(&mut m, &b, t, 1, up(Key::MetaRight)),
            vec![start(HotkeyMode::Toggle)]
        );
        event(&mut m, &b, t, 2, down(Key::MetaRight));
        assert!(event(&mut m, &b, t, 3, up(Key::MetaRight)).is_empty());
        event(&mut m, &b, t, 4, down(Key::F2));
        assert_eq!(
            event(&mut m, &b, t, 5, up(Key::F2)),
            vec![HotkeyAction::StopRecording { prompt_id: None }]
        );
    }
    #[test]
    fn hold_keys_do_not_start_or_stop_during_toggle() {
        let mut b = toggle();
        b.extend(hold(&[Key::ControlLeft]));
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::MetaRight));
        event(&mut m, &b, t, 1, up(Key::MetaRight));
        event(&mut m, &b, t, 2, down(Key::ControlLeft));
        assert!(m.tick(t + Duration::from_secs(2)).is_empty());
        assert!(event(&mut m, &b, t, 2100, up(Key::ControlLeft)).is_empty());
        assert_eq!(m.recording_mode(), Some(HotkeyMode::Toggle));
    }
    #[test]
    fn cancel_and_reconfigure_revoke_pending_timers() {
        let b = hold(&[Key::ControlLeft]);
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::ControlLeft));
        m.cancel_pending();
        assert!(m.tick(t + Duration::from_secs(1)).is_empty());
        event(&mut m, &b, t, 1100, up(Key::ControlLeft));
        event(&mut m, &b, t, 1200, down(Key::ControlLeft));
        m.reset_recording();
        assert!(m.tick(t + Duration::from_secs(3)).is_empty());
    }
    #[test]
    fn recovery_preserves_held_keys_and_never_synthesizes_output() {
        let b = hold(&[Key::ControlLeft]);
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::ControlLeft));
        assert!(!m.recover_released(&[]));
        assert_eq!(
            m.tick(t + Duration::from_secs(3)),
            vec![start(HotkeyMode::Hold)]
        );
        assert!(m.recover_released(&[Key::ControlLeft]));
        m.reset_recording();
        assert!(event(&mut m, &b, t, 3100, up(Key::ControlLeft)).is_empty());
    }
    #[test]
    fn lock_keys_invalidate_without_sticking_the_next_cycle() {
        let b = toggle();
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::MetaRight));
        event(&mut m, &b, t, 1, down(Key::CapsLock));
        assert!(event(&mut m, &b, t, 2, up(Key::MetaRight)).is_empty());
        event(&mut m, &b, t, 3, down(Key::MetaRight));
        assert_eq!(
            event(&mut m, &b, t, 4, up(Key::MetaRight)),
            vec![start(HotkeyMode::Toggle)]
        );
    }
    #[test]
    fn active_toggle_keeps_original_stop_bindings_after_reload() {
        let b = toggle();
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::MetaRight));
        event(&mut m, &b, t, 1, up(Key::MetaRight));
        m.cancel_pending();
        event(&mut m, &[], t, 2, down(Key::MetaRight));
        assert_eq!(
            event(&mut m, &[], t, 3, up(Key::MetaRight)),
            vec![HotkeyAction::StopRecording { prompt_id: None }]
        );
    }
    #[test]
    fn native_modifiers_prevent_stale_callback_from_swallowing_ctrl_f() {
        let b = vec![
            binding(&[Key::F13], HotkeyMode::Toggle, None, true),
            binding(&[Key::F], HotkeyMode::Toggle, Some("p"), false),
        ];
        let mut m = MatcherState::new();
        let t = Instant::now();
        event(&mut m, &b, t, 0, down(Key::F13));
        event(&mut m, &b, t, 1, up(Key::F13));
        assert!(m.should_swallow(Key::F, &b, &[]));
        assert!(!m.should_swallow(Key::F, &b, &[(Key::ControlLeft, true)]));
    }
}
