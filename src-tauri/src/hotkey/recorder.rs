//! Backend hotkey recording (UI recorder): capture a live key combination and
//! build its canonical config string.

use keytap::EventKind;
use keytap::Key;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::HotkeyConfig;

/// Canonical modifier order, matching the frontend recorder: Control, then
/// Alt, Shift, Meta, then Fn last. Also defines the set of held-state modifiers
/// (used by [`is_modifier_key`]) — excludes `CapsLock` for frontend parity.
/// Centralized so the order list and the membership test can't drift apart.
const MODIFIER_ORDER: &[Key] = &[
    Key::ControlLeft,
    Key::ControlRight,
    Key::AltLeft,
    Key::AltRight,
    Key::ShiftLeft,
    Key::ShiftRight,
    Key::MetaLeft,
    Key::MetaRight,
    Key::Function,
];

/// Whether a [`Key`] is a held-state modifier. Includes [`Key::Function`]
/// (the macOS Fn key surfaces as a FlagsChanged modifier) but excludes
/// [`Key::CapsLock`], matching the frontend recorder's classification so a
/// recorded Fn is treated as a modifier-only hotkey.
fn is_modifier_key(key: Key) -> bool {
    MODIFIER_ORDER.contains(&key)
}

/// A finalized recording: the canonical config string plus the first key
/// pressed, which main-hotkey callers validate (see [`is_valid_main_first_key`]).
pub(crate) struct RecordedCombination {
    pub combo: String,
    pub first_key: Key,
}

/// Whether `key` may be the first key of a *main* hotkey. The main hotkey
/// fires on a bare press, so a typing key first (a letter, digit, space, …)
/// would leak its character into the focused app and fire on ordinary
/// typing; non-typing keys — modifiers, Tab, the F row — do neither. Prompt
/// stop keys are exempt: they only act while a recording runs, so their
/// recordings are not validated against this rule.
pub(crate) fn is_valid_main_first_key(key: Key) -> bool {
    is_modifier_key(key) || key == Key::Tab || is_function_key(key)
}

/// Whether `key` is in the F row (F1–F24, matching what the recorder can
/// spell). Enumerated because `Key` has data-carrying variants and cannot be
/// range-matched.
fn is_function_key(key: Key) -> bool {
    matches!(
        key,
        Key::F1
            | Key::F2
            | Key::F3
            | Key::F4
            | Key::F5
            | Key::F6
            | Key::F7
            | Key::F8
            | Key::F9
            | Key::F10
            | Key::F11
            | Key::F12
            | Key::F13
            | Key::F14
            | Key::F15
            | Key::F16
            | Key::F17
            | Key::F18
            | Key::F19
            | Key::F20
            | Key::F21
            | Key::F22
            | Key::F23
            | Key::F24
    )
}

/// Canonical modifier display/sort rank from [`MODIFIER_ORDER`]. Non-modifiers
/// get `u8::MAX` (sort last), but this is only ever called on keys that pass
/// [`is_modifier_key`].
fn modifier_rank(key: Key) -> u8 {
    MODIFIER_ORDER
        .iter()
        .position(|&k| k == key)
        .map(|i| i as u8)
        .unwrap_or(u8::MAX)
}

/// Inverse of [`super::parse::parse_single_key`]: map a recorded [`Key`] to the
/// canonical config token the parser accepts, so a recorded combination
/// round-trips through the config string. Returns `None` for keys with no
/// parseable spelling (numpad keys, raw unknown scancodes).
fn key_to_token(key: Key) -> Option<&'static str> {
    Some(match key {
        // Modifiers
        Key::ControlLeft => "ControlLeft",
        Key::ControlRight => "ControlRight",
        Key::ShiftLeft => "ShiftLeft",
        Key::ShiftRight => "ShiftRight",
        Key::AltLeft => "AltLeft",
        Key::AltRight => "AltRight",
        Key::MetaLeft => "MetaLeft",
        Key::MetaRight => "MetaRight",
        Key::Function => "Fn",
        Key::CapsLock => "CapsLock",
        // Special keys
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Backspace => "Backspace",
        // Arrows
        Key::ArrowUp => "ArrowUp",
        Key::ArrowDown => "ArrowDown",
        Key::ArrowLeft => "ArrowLeft",
        Key::ArrowRight => "ArrowRight",
        // Navigation
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Insert => "Insert",
        Key::Delete => "Delete",
        // Function row
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::F13 => "F13",
        Key::F14 => "F14",
        Key::F15 => "F15",
        Key::F16 => "F16",
        Key::F17 => "F17",
        Key::F18 => "F18",
        Key::F19 => "F19",
        Key::F20 => "F20",
        Key::F21 => "F21",
        Key::F22 => "F22",
        Key::F23 => "F23",
        Key::F24 => "F24",
        // Letters (single uppercase chars, as parse_single_key expects)
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        // Digits (single-char tokens)
        Key::Digit0 => "0",
        Key::Digit1 => "1",
        Key::Digit2 => "2",
        Key::Digit3 => "3",
        Key::Digit4 => "4",
        Key::Digit5 => "5",
        Key::Digit6 => "6",
        Key::Digit7 => "7",
        Key::Digit8 => "8",
        Key::Digit9 => "9",
        // Punctuation
        Key::Backtick => "Backtick",
        Key::Minus => "Minus",
        Key::Equal => "Equal",
        Key::BracketLeft => "BracketLeft",
        Key::BracketRight => "BracketRight",
        Key::Backslash => "Backslash",
        Key::Semicolon => "Semicolon",
        Key::Quote => "Quote",
        Key::Comma => "Comma",
        Key::Period => "Period",
        Key::Slash => "Slash",
        Key::IntlBackslash => "IntlBackslash",
        // Misc
        Key::Menu => "Menu",
        Key::PrintScreen => "PrintScreen",
        Key::ScrollLock => "ScrollLock",
        Key::Pause => "Pause",
        Key::NumLock => "NumLock",
        // Numpad keys and Unknown scancodes have no parseable token
        _ => return None,
    })
}

/// Build the config string (e.g. "ControlLeft+A", "Fn") for a set of held
/// keys, mirroring the frontend recorder: modifiers in canonical order, then
/// the main key if any.
fn build_hotkey_string(pressed: &BTreeSet<Key>) -> Option<String> {
    let mut mods: Vec<Key> = Vec::new();
    let mut main_key: Option<Key> = None;
    for &k in pressed {
        if is_modifier_key(k) {
            mods.push(k);
        } else if main_key.is_none() {
            main_key = Some(k);
        }
    }
    mods.sort_by_key(|&k| modifier_rank(k));

    let mut parts: Vec<&str> = mods.iter().filter_map(|&k| key_to_token(k)).collect();
    if let Some(mk) = main_key {
        if let Some(tok) = key_to_token(mk) {
            parts.push(tok);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("+"))
    }
}

/// Finalize a recording from the peak pressed set plus the first key pressed.
/// `first` is always `Some` by the time a combination finalizes (every set
/// member arrived through a KeyDown); `None` pairs only occur when the
/// combination itself has no parseable spelling.
fn finalize(pressed: &BTreeSet<Key>, first: Option<Key>) -> Option<RecordedCombination> {
    match (build_hotkey_string(pressed), first) {
        (Some(combo), Some(first_key)) => Some(RecordedCombination { combo, first_key }),
        _ => None,
    }
}

/// Record one hotkey combination from the live keyboard. Mirrors the frontend
/// DOM recorder's state machine so behaviour stays consistent:
///   - keydown accumulates into the pressed set;
///   - releasing a non-modifier finalizes "modifiers + that key";
///   - releasing a modifier when only modifiers are held starts a 300ms timer
///     that finalizes a modifier-only hotkey (e.g. `Fn`, `RightShift`);
///   - `Escape` cancels; elapsed `timeout` with no completion cancels.
///
/// Runs on a dedicated blocking thread; returns `None` on cancel/timeout, else
/// the combination plus the first key pressed (main-hotkey callers validate
/// it via [`is_valid_main_first_key`]).
///
/// All platforms consume events relayed from the resident keytap listener.
/// The listener receives captured events even when foreground delivery is
/// suppressed; no second tap or keyboard-state polling is needed.
pub(crate) fn record_combination(
    config: &HotkeyConfig,
    timeout: Duration,
    session: &str,
) -> Option<RecordedCombination> {
    let start = Instant::now();
    let (tx, rx) = crossbeam_channel::bounded::<keytap::Event>(256);
    let tap_active = {
        let mut cfg = config.write().unwrap();
        cfg.record_tx = Some(tx);
        cfg.recording = true;
        cfg.tap_active
    };

    struct RelayGuard<'a> {
        config: &'a HotkeyConfig,
    }
    impl Drop for RelayGuard<'_> {
        fn drop(&mut self) {
            let mut cfg = self.config.write().unwrap();
            cfg.record_tx = None;
            cfg.recording = false;
        }
    }
    let guard = RelayGuard { config };
    log_hotkey!(
        info,
        "recorder session={} stage=start source=keytap tap_active={} timeout_ms={}",
        session,
        tap_active,
        timeout.as_millis()
    );
    let mut event_count = 0;
    let result = capture_events(&rx, timeout, start, session, &mut event_count);
    drop(guard);
    log_hotkey!(
        info,
        "recorder session={} stage=end elapsed_ms={} events={} combo={:?} first_key={:?}",
        session,
        start.elapsed().as_millis(),
        event_count,
        result.as_ref().map(|r| r.combo.as_str()),
        result.as_ref().map(|r| r.first_key)
    );
    result
}

fn capture_events(
    rx: &crossbeam_channel::Receiver<keytap::Event>,
    timeout: Duration,
    start: Instant,
    session: &str,
    event_count: &mut usize,
) -> Option<RecordedCombination> {
    let mut pressed: BTreeSet<Key> = BTreeSet::new();
    let mut first_pressed: Option<Key> = None;
    let mut finalize_at: Option<Instant> = None;

    loop {
        if start.elapsed() >= timeout {
            log_hotkey!(
                info,
                "recorder session={} outcome=timeout pressed={:?}",
                session,
                pressed
            );
            return None;
        }
        if let Some(deadline) = finalize_at {
            if Instant::now() >= deadline {
                finalize_at = None;
                if !pressed.is_empty() && pressed.iter().all(|&k| is_modifier_key(k)) {
                    log_hotkey!(
                        info,
                        "recorder session={} outcome=modifier_complete",
                        session
                    );
                    return finalize(&pressed, first_pressed);
                }
            }
        }

        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => {
                *event_count += 1;
                // Log before any cancellation, token conversion or validation.
                log_hotkey!(
                    info,
                    "recorder session={} seq={} stage=consume elapsed_ms={} event={:?} event_age_ms={}",
                    session,
                    event_count,
                    start.elapsed().as_millis(),
                    event.kind,
                    event.time.elapsed().as_millis()
                );
                match event.kind {
                    EventKind::KeyDown(Key::Escape) => {
                        log_hotkey!(info, "recorder session={} outcome=escape", session);
                        return None;
                    }
                    EventKind::KeyDown(k) => {
                        pressed.insert(k);
                        first_pressed.get_or_insert(k);
                        if key_to_token(k).is_none() {
                            log_hotkey!(
                                info,
                                "recorder session={} key={:?} token=unsupported",
                                session,
                                k
                            );
                        }
                        if !is_modifier_key(k) {
                            finalize_at = None;
                        }
                    }
                    EventKind::KeyUp(k) => {
                        if !is_modifier_key(k) {
                            if let Some(recorded) = finalize(&pressed, first_pressed) {
                                log_hotkey!(
                                    info,
                                    "recorder session={} outcome=key_complete",
                                    session
                                );
                                return Some(recorded);
                            }
                            log_hotkey!(
                                info,
                                "recorder session={} stage=finalize outcome=no_parseable_combination pressed={:?}",
                                session,
                                pressed
                            );
                        } else if !pressed.is_empty() && pressed.iter().all(|&k| is_modifier_key(k))
                        {
                            finalize_at = Some(Instant::now() + Duration::from_millis(300));
                        }
                    }
                    EventKind::KeyRepeat(_) => {
                        log_hotkey!(info, "recorder session={} action=ignore_repeat", session);
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                log_hotkey!(
                    warn,
                    "recorder session={} outcome=relay_disconnected",
                    session
                );
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keytap::Key;
    use std::collections::BTreeSet;

    // parse_hotkey_string lives in the sibling `parse` module; re-import for
    // the round-trip test so we don't reach across via fragile paths.
    use super::super::parse::parse_hotkey_string;

    fn capture(kinds: &[EventKind]) -> Option<RecordedCombination> {
        let (tx, rx) = crossbeam_channel::unbounded();
        for &kind in kinds {
            tx.send(keytap::Event {
                time: Instant::now(),
                kind,
            })
            .unwrap();
        }
        let mut count = 0;
        capture_events(
            &rx,
            Duration::from_secs(1),
            Instant::now(),
            "test",
            &mut count,
        )
    }

    #[test]
    fn event_capture_preserves_first_key_order_and_ignores_repeats() {
        for (first, second) in [(Key::F, Key::ControlLeft), (Key::ControlLeft, Key::F)] {
            let result = capture(&[
                EventKind::KeyDown(first),
                EventKind::KeyRepeat(first),
                EventKind::KeyDown(second),
                EventKind::KeyUp(Key::F),
            ])
            .unwrap();
            assert_eq!(result.combo, "ControlLeft+F");
            assert_eq!(result.first_key, first);
        }
    }

    #[test]
    fn event_capture_completes_modifier_only_combination() {
        let result = capture(&[
            EventKind::KeyDown(Key::ControlRight),
            EventKind::KeyDown(Key::ShiftRight),
            EventKind::KeyUp(Key::ShiftRight),
            EventKind::KeyUp(Key::ControlRight),
        ])
        .unwrap();
        assert_eq!(result.combo, "ControlRight+ShiftRight");
        assert_eq!(result.first_key, Key::ControlRight);
    }

    #[test]
    fn event_capture_cancels_on_escape_and_accepts_typing_keys_before_validation() {
        assert!(capture(&[
            EventKind::KeyDown(Key::ControlLeft),
            EventKind::KeyDown(Key::Escape),
        ])
        .is_none());
        let result = capture(&[EventKind::KeyDown(Key::A), EventKind::KeyUp(Key::A)]).unwrap();
        assert_eq!(result.combo, "A");
        assert!(!is_valid_main_first_key(result.first_key));
    }

    #[test]
    fn event_capture_reports_unrecognized_keys_without_producing_a_combination() {
        let key = Key::Unknown(keytap::RawCode(42));
        let (tx, rx) = crossbeam_channel::unbounded();
        for kind in [EventKind::KeyDown(key), EventKind::KeyUp(key)] {
            tx.send(keytap::Event {
                time: Instant::now(),
                kind,
            })
            .unwrap();
        }
        drop(tx);
        let mut count = 0;
        assert!(capture_events(
            &rx,
            Duration::from_secs(1),
            Instant::now(),
            "test",
            &mut count
        )
        .is_none());
        assert_eq!(count, 2);
    }

    #[test]
    fn recording_timeout_clears_the_relay_and_recording_flag() {
        let config = super::super::create_config(vec![]);
        assert!(record_combination(&config, Duration::ZERO, "test").is_none());
        let cfg = config.read().unwrap();
        assert!(!cfg.recording);
        assert!(cfg.record_tx.is_none());
    }

    #[test]
    fn recording_relay_delivers_events_and_cleans_up_after_success_or_escape() {
        for cancel in [false, true] {
            let config = super::super::create_config(vec![]);
            let sender_config = config.clone();
            let producer = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                let tx = loop {
                    {
                        let cfg = sender_config.read().unwrap();
                        if cfg.recording {
                            break cfg
                                .record_tx
                                .clone()
                                .expect("recording must have a receiver");
                        }
                    }
                    assert!(Instant::now() < deadline, "recorder never started");
                    std::thread::sleep(Duration::from_millis(1));
                };
                let kinds = if cancel {
                    vec![EventKind::KeyDown(Key::Escape)]
                } else {
                    vec![EventKind::KeyDown(Key::F8), EventKind::KeyUp(Key::F8)]
                };
                for kind in kinds {
                    tx.send(keytap::Event {
                        time: Instant::now(),
                        kind,
                    })
                    .unwrap();
                }
            });
            let result = record_combination(&config, Duration::from_secs(2), "test");
            producer.join().unwrap();
            if cancel {
                assert!(result.is_none());
            } else {
                assert_eq!(result.unwrap().combo, "F8");
            }
            let cfg = config.read().unwrap();
            assert!(!cfg.recording);
            assert!(cfg.record_tx.is_none());
        }
    }

    #[test]
    fn key_to_token_round_trips_through_parser() {
        // Every token key_to_token emits must parse back to the same Key —
        // otherwise the recorder would write config strings it can't read.
        let cases = [
            Key::ControlLeft,
            Key::ControlRight,
            Key::ShiftLeft,
            Key::ShiftRight,
            Key::AltLeft,
            Key::AltRight,
            Key::MetaLeft,
            Key::MetaRight,
            Key::Function,
            Key::CapsLock,
            Key::Space,
            Key::Enter,
            Key::Escape,
            Key::Backspace,
            Key::ArrowUp,
            Key::ArrowDown,
            Key::Home,
            Key::PageDown,
            Key::F1,
            Key::F13,
            Key::F24,
            Key::A,
            Key::Z,
            Key::Digit0,
            Key::Digit9,
            Key::Minus,
            Key::Slash,
            Key::IntlBackslash,
            Key::NumLock,
        ];
        for k in cases {
            let tok = key_to_token(k).unwrap_or_else(|| panic!("{k:?} has no token"));
            let parsed =
                parse_hotkey_string(tok).unwrap_or_else(|| panic!("token '{tok}' did not parse"));
            assert_eq!(
                parsed,
                BTreeSet::from([k]),
                "round-trip failed for {k:?} -> '{tok}'"
            );
        }
    }

    #[test]
    fn key_to_token_returns_none_for_unparsable() {
        assert_eq!(key_to_token(Key::Unknown(keytap::RawCode(42))), None);
        assert_eq!(key_to_token(Key::Numpad0), None);
    }

    #[test]
    fn is_modifier_key_classification() {
        assert!(is_modifier_key(Key::Function));
        assert!(is_modifier_key(Key::ControlLeft));
        assert!(is_modifier_key(Key::MetaRight));
        // CapsLock is NOT treated as a held modifier here (frontend parity).
        assert!(!is_modifier_key(Key::CapsLock));
        assert!(!is_modifier_key(Key::A));
        assert!(!is_modifier_key(Key::Space));
    }

    #[test]
    fn build_hotkey_string_sorts_modifiers_then_main_key() {
        // Press order is A, Shift, Control — output must be canonical order.
        let pressed = BTreeSet::from([Key::A, Key::ShiftLeft, Key::ControlLeft]);
        assert_eq!(
            build_hotkey_string(&pressed).as_deref(),
            Some("ControlLeft+ShiftLeft+A")
        );
    }

    #[test]
    fn build_hotkey_string_modifier_only_fn() {
        let pressed = BTreeSet::from([Key::Function]);
        assert_eq!(build_hotkey_string(&pressed).as_deref(), Some("Fn"));
    }

    #[test]
    fn build_hotkey_string_single_main_key() {
        let pressed = BTreeSet::from([Key::F13]);
        assert_eq!(build_hotkey_string(&pressed).as_deref(), Some("F13"));
    }

    #[test]
    fn valid_main_first_keys() {
        // Modifiers (incl. Fn), Tab, and the whole F row are allowed.
        for key in [
            Key::ControlLeft,
            Key::MetaRight,
            Key::ShiftLeft,
            Key::AltLeft,
            Key::Function,
            Key::Tab,
            Key::F1,
            Key::F12,
            Key::F13,
            Key::F24,
        ] {
            assert!(is_valid_main_first_key(key), "{key:?} should be valid");
        }
    }

    #[test]
    fn invalid_main_first_keys() {
        // Typing keys are rejected — they would leak into the focused app.
        for key in [
            Key::F,
            Key::A,
            Key::Digit5,
            Key::Space,
            Key::Enter,
            Key::Quote,
            Key::Numpad0,
        ] {
            assert!(!is_valid_main_first_key(key), "{key:?} should be invalid");
        }
    }

    #[test]
    fn finalize_pairs_combo_with_first_pressed_key() {
        // Press order F then Shift: the combo canonicalizes, the first key
        // records the (rejected-for-main) typing key.
        let pressed = BTreeSet::from([Key::F, Key::ShiftLeft]);
        let recorded = finalize(&pressed, Some(Key::F)).unwrap();
        assert_eq!(recorded.combo, "ShiftLeft+F");
        assert_eq!(recorded.first_key, Key::F);

        // Unparsable combinations finalize to None even with a first key.
        let unparsable = BTreeSet::from([Key::Numpad0]);
        assert!(finalize(&unparsable, Some(Key::Numpad0)).is_none());
    }
}
