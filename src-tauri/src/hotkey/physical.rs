//! Native key-state probes used only for recovery, never as the hotkey event source.
use keytap::Key;

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
}

/// None means unsupported/unknown: callers must preserve the observed key state.
pub(super) fn is_pressed(key: Key) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        // Modifiers arrive as FlagsChanged, not ordinary keyboard events. In
        // particular, right-side modifiers must not be reclaimed using the
        // ordinary key-state table before their hold deadline.
        if is_macos_modifier(key) {
            return macos_modifier_pressed(key, unsafe { CGEventSourceFlagsState(1) });
        }
        let code = match key {
            Key::A => 0,
            Key::B => 11,
            Key::C => 8,
            Key::D => 2,
            Key::E => 14,
            Key::F => 3,
            Key::G => 5,
            Key::H => 4,
            Key::I => 34,
            Key::J => 38,
            Key::K => 40,
            Key::L => 37,
            Key::M => 46,
            Key::N => 45,
            Key::O => 31,
            Key::P => 35,
            Key::Q => 12,
            Key::R => 15,
            Key::S => 1,
            Key::T => 17,
            Key::U => 32,
            Key::V => 9,
            Key::W => 13,
            Key::X => 7,
            Key::Y => 16,
            Key::Z => 6,
            Key::Digit0 => 29,
            Key::Digit1 => 18,
            Key::Digit2 => 19,
            Key::Digit3 => 20,
            Key::Digit4 => 21,
            Key::Digit5 => 23,
            Key::Digit6 => 22,
            Key::Digit7 => 26,
            Key::Digit8 => 28,
            Key::Digit9 => 25,
            Key::Minus => 27,
            Key::Equal => 24,
            Key::BracketLeft => 33,
            Key::BracketRight => 30,
            Key::Backslash => 42,
            Key::Semicolon => 41,
            Key::Quote => 39,
            Key::Comma => 43,
            Key::Period => 47,
            Key::Slash => 44,
            Key::Backtick => 50,
            Key::ShiftLeft => 56,
            Key::ShiftRight => 60,
            Key::ControlLeft => 59,
            Key::ControlRight => 62,
            Key::AltLeft => 58,
            Key::AltRight => 61,
            Key::MetaLeft => 55,
            Key::MetaRight => 54,
            Key::Enter => 36,
            Key::Tab => 48,
            Key::Space => 49,
            Key::Backspace => 51,
            Key::Escape => 53,
            Key::ArrowLeft => 123,
            Key::ArrowRight => 124,
            Key::ArrowDown => 125,
            Key::ArrowUp => 126,
            Key::Home => 115,
            Key::End => 119,
            Key::PageUp => 116,
            Key::PageDown => 121,
            Key::Delete => 117,
            Key::F1 => 122,
            Key::F2 => 120,
            Key::F3 => 99,
            Key::F4 => 118,
            Key::F5 => 96,
            Key::F6 => 97,
            Key::F7 => 98,
            Key::F8 => 100,
            Key::F9 => 101,
            Key::F10 => 109,
            Key::F11 => 103,
            Key::F12 => 111,
            Key::F13 => 105,
            Key::F14 => 107,
            Key::F15 => 113,
            Key::F16 => 106,
            Key::F17 => 64,
            Key::F18 => 79,
            Key::F19 => 80,
            Key::F20 => 90,
            Key::Numpad0 => 82,
            Key::Numpad1 => 83,
            Key::Numpad2 => 84,
            Key::Numpad3 => 85,
            Key::Numpad4 => 86,
            Key::Numpad5 => 87,
            Key::Numpad6 => 88,
            Key::Numpad7 => 89,
            Key::Numpad8 => 91,
            Key::Numpad9 => 92,
            Key::NumpadAdd => 69,
            Key::NumpadSubtract => 78,
            Key::NumpadMultiply => 67,
            Key::NumpadDivide => 75,
            Key::NumpadEnter => 76,
            Key::NumpadDecimal => 65,
            Key::IntlBackslash => 10,
            Key::Function => 63,
            Key::Unknown(raw) => u16::try_from(raw.0).ok()?,
            _ => return None,
        };
        // HID system state is independent of foreground key suppression.
        Some(unsafe { CGEventSourceKeyState(1, code) })
    }
    #[cfg(target_os = "windows")]
    {
        let code = match key {
            Key::ShiftLeft => 0xA0,
            Key::ShiftRight => 0xA1,
            Key::ControlLeft => 0xA2,
            Key::ControlRight => 0xA3,
            Key::AltLeft => 0xA4,
            Key::AltRight => 0xA5,
            Key::MetaLeft => 0x5B,
            Key::MetaRight => 0x5C,
            Key::Backspace => 0x08,
            Key::Tab => 0x09,
            Key::Enter => 0x0D,
            Key::Escape => 0x1B,
            Key::Space => 0x20,
            Key::ArrowLeft => 0x25,
            Key::ArrowUp => 0x26,
            Key::ArrowRight => 0x27,
            Key::ArrowDown => 0x28,
            Key::Home => 0x24,
            Key::End => 0x23,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Insert => 0x2D,
            Key::Delete => 0x2E,
            Key::PrintScreen => 0x2C,
            Key::Pause => 0x13,
            Key::Menu => 0x5D,
            Key::Digit0 => 0x30,
            Key::Digit1 => 0x31,
            Key::Digit2 => 0x32,
            Key::Digit3 => 0x33,
            Key::Digit4 => 0x34,
            Key::Digit5 => 0x35,
            Key::Digit6 => 0x36,
            Key::Digit7 => 0x37,
            Key::Digit8 => 0x38,
            Key::Digit9 => 0x39,
            Key::A => 0x41,
            Key::B => 0x42,
            Key::C => 0x43,
            Key::D => 0x44,
            Key::E => 0x45,
            Key::F => 0x46,
            Key::G => 0x47,
            Key::H => 0x48,
            Key::I => 0x49,
            Key::J => 0x4A,
            Key::K => 0x4B,
            Key::L => 0x4C,
            Key::M => 0x4D,
            Key::N => 0x4E,
            Key::O => 0x4F,
            Key::P => 0x50,
            Key::Q => 0x51,
            Key::R => 0x52,
            Key::S => 0x53,
            Key::T => 0x54,
            Key::U => 0x55,
            Key::V => 0x56,
            Key::W => 0x57,
            Key::X => 0x58,
            Key::Y => 0x59,
            Key::Z => 0x5A,
            Key::F1 => 0x70,
            Key::F2 => 0x71,
            Key::F3 => 0x72,
            Key::F4 => 0x73,
            Key::F5 => 0x74,
            Key::F6 => 0x75,
            Key::F7 => 0x76,
            Key::F8 => 0x77,
            Key::F9 => 0x78,
            Key::F10 => 0x79,
            Key::F11 => 0x7A,
            Key::F12 => 0x7B,
            Key::F13 => 0x7C,
            Key::F14 => 0x7D,
            Key::F15 => 0x7E,
            Key::F16 => 0x7F,
            Key::F17 => 0x80,
            Key::F18 => 0x81,
            Key::F19 => 0x82,
            Key::F20 => 0x83,
            Key::F21 => 0x84,
            Key::F22 => 0x85,
            Key::F23 => 0x86,
            Key::F24 => 0x87,
            Key::Numpad0 => 0x60,
            Key::Numpad1 => 0x61,
            Key::Numpad2 => 0x62,
            Key::Numpad3 => 0x63,
            Key::Numpad4 => 0x64,
            Key::Numpad5 => 0x65,
            Key::Numpad6 => 0x66,
            Key::Numpad7 => 0x67,
            Key::Numpad8 => 0x68,
            Key::Numpad9 => 0x69,
            Key::NumpadMultiply => 0x6A,
            Key::NumpadAdd => 0x6B,
            Key::NumpadSubtract => 0x6D,
            Key::NumpadDecimal => 0x6E,
            Key::NumpadDivide => 0x6F,
            Key::Semicolon => 0xBA,
            Key::Equal => 0xBB,
            Key::Comma => 0xBC,
            Key::Minus => 0xBD,
            Key::Period => 0xBE,
            Key::Slash => 0xBF,
            Key::Backtick => 0xC0,
            Key::BracketLeft => 0xDB,
            Key::Backslash => 0xDC,
            Key::BracketRight => 0xDD,
            Key::Quote => 0xDE,
            Key::IntlBackslash => 0xE2,
            Key::NumpadEnter => 0x0D,
            Key::Unknown(raw) => i32::try_from(raw.0).ok()?,
            _ => return None,
        };
        Some(unsafe { GetAsyncKeyState(code) } < 0)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = key;
        None
    }
}

/// Current native modifier states close the callback/listener ordering gap:
/// a fast Ctrl+F must not be swallowed as a bare F stop key.
pub(super) fn modifier_states() -> Vec<(Key, bool)> {
    [
        Key::ControlLeft,
        Key::ControlRight,
        Key::ShiftLeft,
        Key::ShiftRight,
        Key::AltLeft,
        Key::AltRight,
        Key::MetaLeft,
        Key::MetaRight,
        Key::Function,
    ]
    .into_iter()
    .filter_map(|key| is_pressed(key).map(|pressed| (key, pressed)))
    .collect()
}

// Device-specific bits from IOKit/hidsystem/IOLLEvent.h. Generic flags alone
// cannot distinguish left/right; preserve event-stream state if those bits
// are unavailable rather than inventing a release on one side.
#[cfg(any(target_os = "macos", test))]
fn is_macos_modifier(key: Key) -> bool {
    super::matcher::is_modifier(key)
}

#[cfg(any(target_os = "macos", test))]
fn macos_modifier_pressed(key: Key, flags: u64) -> Option<bool> {
    let (side, opposite, generic) = match key {
        Key::ControlLeft => (0x1, 0x2000, 0x40000),
        Key::ControlRight => (0x2000, 0x1, 0x40000),
        Key::ShiftLeft => (0x2, 0x4, 0x20000),
        Key::ShiftRight => (0x4, 0x2, 0x20000),
        Key::AltLeft => (0x20, 0x40, 0x80000),
        Key::AltRight => (0x40, 0x20, 0x80000),
        Key::MetaLeft => (0x8, 0x10, 0x100000),
        Key::MetaRight => (0x10, 0x8, 0x100000),
        Key::Function => return Some(flags & 0x800000 != 0),
        _ => return None,
    };
    if flags & side != 0 {
        Some(true)
    } else if flags & generic == 0 || flags & opposite != 0 {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::matcher::{HotkeyAction, HOLD_DELAY};
    use crate::hotkey::{HotkeyBinding, HotkeyMode, MatcherState};
    use std::time::Instant;

    #[test]
    fn macos_flags_distinguish_both_sides_and_generic_only_is_unknown() {
        for (left, right, generic, left_bit, right_bit) in [
            (Key::MetaLeft, Key::MetaRight, 0x100000, 0x8, 0x10),
            (Key::AltLeft, Key::AltRight, 0x80000, 0x20, 0x40),
            (Key::ControlLeft, Key::ControlRight, 0x40000, 0x1, 0x2000),
            (Key::ShiftLeft, Key::ShiftRight, 0x20000, 0x2, 0x4),
        ] {
            assert!(is_macos_modifier(right));
            assert_eq!(
                macos_modifier_pressed(right, generic | right_bit),
                Some(true)
            );
            assert_eq!(
                macos_modifier_pressed(left, generic | right_bit),
                Some(false)
            );
            assert_eq!(macos_modifier_pressed(left, generic | left_bit), Some(true));
            assert_eq!(
                macos_modifier_pressed(right, generic | left_bit),
                Some(false)
            );
            assert_eq!(
                macos_modifier_pressed(right, generic | left_bit | right_bit),
                Some(true)
            );
            assert_eq!(macos_modifier_pressed(right, generic), None);
            assert_eq!(macos_modifier_pressed(right, 0), Some(false));
        }
    }
    #[test]
    fn right_modifier_hold_survives_deadline_probe_and_stops_on_release() {
        for (key, flags) in [
            (Key::MetaRight, 0x100010),
            (Key::AltRight, 0x80040),
            (Key::ControlRight, 0x42000),
        ] {
            let binding = HotkeyBinding {
                keys: [key].into_iter().collect(),
                mode: HotkeyMode::Hold,
                prompt_id: Some("p".into()),
                wake_only: false,
            };
            let mut matcher = MatcherState::new();
            let now = Instant::now();
            matcher.process_at(
                keytap::EventKind::KeyDown(key),
                std::slice::from_ref(&binding),
                now,
            );
            let released: Vec<_> = [key]
                .into_iter()
                .filter(|key| macos_modifier_pressed(*key, flags) == Some(false))
                .collect();
            assert!(!matcher.recover_released(&released));
            assert!(matches!(
                &matcher.tick(now + HOLD_DELAY)[..],
                [HotkeyAction::StartRecording {
                    mode: HotkeyMode::Hold,
                    ..
                }]
            ));
            assert_eq!(
                matcher.process_at(keytap::EventKind::KeyUp(key), &[binding], now + HOLD_DELAY),
                vec![HotkeyAction::StopRecording {
                    prompt_id: Some("p".into())
                }]
            );
        }
    }
    #[test]
    fn confirmed_release_cancels_pending_hold_but_unknown_state_does_not() {
        for flags in [0, 0x100000] {
            let key = Key::MetaRight;
            let binding = HotkeyBinding {
                keys: [key].into_iter().collect(),
                mode: HotkeyMode::Hold,
                prompt_id: None,
                wake_only: false,
            };
            let mut matcher = MatcherState::new();
            let now = Instant::now();
            matcher.process_at(keytap::EventKind::KeyDown(key), &[binding], now);
            let released: Vec<_> = [key]
                .into_iter()
                .filter(|key| macos_modifier_pressed(*key, flags) == Some(false))
                .collect();
            matcher.recover_released(&released);
            assert_eq!(matcher.tick(now + HOLD_DELAY).is_empty(), flags == 0);
        }
    }
}
