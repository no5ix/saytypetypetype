//! Hotkey display labels for the settings UI.

use std::sync::Arc;

use crate::app_state::AppInner;

/// Map one accelerator token to the display label the settings UI shows.
/// Mirrors the frontend `normalizeHotkeyLabel` so the overlay label matches
/// system settings. Compiled per target OS: macOS renders Apple symbols
/// (⌃ ⇧ ⌥ ⌘), Windows renders its native labels (Ctrl / Shift / Alt / Win).
pub fn normalize_hotkey_key(key: &str) -> &str {
    // [windows_label, macos_symbol], indexed by the compiled target OS.
    let idx = cfg!(target_os = "macos") as usize;
    match key {
        // CmdOrCtrl resolves to Ctrl on Windows (accelerator convention).
        "CmdOrCtrl" | "CommandOrControl" | "Command" | "Cmd" | "Meta" => ["Ctrl", "⌘"][idx],
        "Control" | "Ctrl" => ["Ctrl", "⌃"][idx],
        "Shift" => ["Shift", "⇧"][idx],
        "Alt" | "Option" => ["Alt", "⌥"][idx],
        "Space" => "␣",
        "ControlLeft" => ["L Ctrl", "L ⌃"][idx],
        "ControlRight" => ["R Ctrl", "R ⌃"][idx],
        "ShiftLeft" => ["L Shift", "L ⇧"][idx],
        "ShiftRight" => ["R Shift", "R ⇧"][idx],
        "AltLeft" => ["L Alt", "L ⌥"][idx],
        "AltRight" => ["R Alt", "R ⌥"][idx],
        "MetaLeft" => ["L Win", "L ⌘"][idx],
        "MetaRight" => ["R Win", "R ⌘"][idx],
        other => other,
    }
}

/// Bare label for a side-specific modifier, `[windows_label, macos_symbol]`,
/// used inside a multi-key chord. The L/R badge is meaningful for a lone key
/// ("R ⌥" is a real choice between two physical keys) and only noise inside a
/// combination, where "L ⌃ L ⌘ L" reads as three L keys.
fn normalize_chord_key(key: &str) -> Option<&'static str> {
    let idx = cfg!(target_os = "macos") as usize;
    match key {
        "ControlLeft" | "ControlRight" => Some(["Ctrl", "⌃"][idx]),
        "ShiftLeft" | "ShiftRight" => Some(["Shift", "⇧"][idx]),
        "AltLeft" | "AltRight" => Some(["Alt", "⌥"][idx]),
        // The physical Meta keys are the Windows logo key, not Ctrl.
        "MetaLeft" | "MetaRight" => Some(["Win", "⌘"][idx]),
        _ => None,
    }
}

/// Format an accelerator string ("AltRight", "Control+Space") into the display
/// label shown in settings. Per-platform via `normalize_hotkey_key`: macOS
/// ("R ⌥", "⌃ ␣"), Windows ("R Alt", "Ctrl ␣"). Inside a chord, sided modifiers
/// lose their L/R badge ("ControlLeft+MetaLeft+L" is "⌃ ⌘ L" / "Ctrl Win L").
pub fn format_hotkey_label(hotkey: &str) -> String {
    let keys: Vec<&str> = hotkey.split('+').map(|k| k.trim()).collect();
    let chord = keys.len() > 1;
    keys.into_iter()
        .map(|k| {
            if chord {
                normalize_chord_key(k).unwrap_or_else(|| normalize_hotkey_key(k))
            } else {
                normalize_hotkey_key(k)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The configured main hotkey, formatted for display. Empty for recorded keycode
/// sequences (which have no stable accelerator string).
pub async fn current_hotkey_label(app_inner: &Arc<AppInner>) -> String {
    let Ok(config) = app_inner.config_manager.load_config() else {
        return String::new();
    };
    match &config.app.hotkey {
        serde_norway::Value::String(s) => format_hotkey_label(s),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_hotkey_label;

    #[test]
    fn function_key_passes_through() {
        assert_eq!(format_hotkey_label("F13"), "F13");
    }

    #[test]
    fn sided_modifier_matches_settings_symbol() {
        // Mirrors the frontend normalizeHotkeyLabel.
        if cfg!(target_os = "macos") {
            assert_eq!(format_hotkey_label("AltRight"), "R ⌥");
        } else {
            assert_eq!(format_hotkey_label("AltRight"), "R Alt");
        }
    }

    /// A chord main key (the shortcut displayed on the result panel's original
    /// card) drops the L/R badges: three of them read as three separate keys.
    #[test]
    fn chord_labels_drop_the_side_badges() {
        if cfg!(target_os = "macos") {
            assert_eq!(format_hotkey_label("ControlLeft+MetaLeft+L"), "⌃ ⌘ L");
            assert_eq!(format_hotkey_label("AltRight+ShiftRight+D"), "⌥ ⇧ D");
        } else {
            assert_eq!(format_hotkey_label("ControlLeft+MetaLeft+L"), "Ctrl Win L");
            assert_eq!(format_hotkey_label("AltRight+ShiftRight+D"), "Alt Shift D");
        }
    }

    #[test]
    fn combo_is_symbolized_and_joined() {
        if cfg!(target_os = "macos") {
            assert_eq!(format_hotkey_label("Control+Space"), "⌃ ␣");
            assert_eq!(format_hotkey_label("CmdOrCtrl+Shift+A"), "⌘ ⇧ A");
        } else {
            assert_eq!(format_hotkey_label("Control+Space"), "Ctrl ␣");
            assert_eq!(format_hotkey_label("CmdOrCtrl+Shift+A"), "Ctrl Shift A");
        }
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(format_hotkey_label(""), "");
    }
}
