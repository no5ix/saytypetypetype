//! One binding compiler for startup, saves, runtime reloads and settings warnings.
use std::collections::HashSet;

use serde::Serialize;

use super::{parse_hotkey_string, recorder, HotkeyBinding, HotkeyMode};
use crate::config::{AppSettings, PromptItem};

#[derive(Debug, Clone, Serialize)]
pub struct BindingIssue {
    pub id: String,
    pub message: String,
    #[serde(skip)]
    fingerprint: String,
}

/// Config id of the implicit stop binding that turns the global main key into a
/// start/stop toggle when no separate raw stop key is configured.
pub const MAIN_TOGGLE_STOP_ID: &str = "main:toggle-stop";

pub fn compile_bindings(
    app: &AppSettings,
    prompts: &[PromptItem],
) -> (Vec<HotkeyBinding>, Vec<BindingIssue>) {
    let main = app.hotkey.as_str().unwrap_or("");
    let voice = app
        .voice_hotkey
        .as_deref()
        .unwrap_or(if app.hotkey_mode == "hold" { "" } else { main });
    let mut specs = vec![
        (
            "main".to_string(),
            "全局主键".to_string(),
            main,
            "toggle",
            None,
            true,
        ),
        (
            "voice".to_string(),
            "语音输入".to_string(),
            voice,
            app.hotkey_mode.as_str(),
            None,
            false,
        ),
    ];
    // The global main key is a complete toggle on its own: in toggle mode the
    // same chord (a modifier combination such as ⌃⌘L, or a single function key)
    // is registered as a stop alias, so a second press ends the recording the
    // first press started. A separately configured stop key works alongside it.
    // In hold mode the main key stays a wake key only — those sessions end on
    // release of their own hold shortcut.
    let main_keys = parse_hotkey_string(main);
    let main_is_wake_capable = main_keys.as_ref().is_some_and(|keys| {
        keys.iter().any(|k| recorder::is_valid_main_first_key(*k))
            && (keys.len() <= 1 || keys.iter().any(|k| !recorder::is_modifier_key(*k)))
    });
    // The default configuration already stores the stop key as the main key
    // itself, and that binding is the alias — registering it twice would be a
    // self-conflict.
    let alias_exists = main_keys.as_ref().is_some_and(|main_keys| {
        parse_hotkey_string(voice).is_some_and(|voice_keys| voice_keys == *main_keys)
    });
    let main_stops_itself = app.hotkey_mode == "toggle" && main_is_wake_capable && !alias_exists;
    if main_stops_itself {
        specs.push((
            MAIN_TOGGLE_STOP_ID.to_string(),
            "全局主键".to_string(),
            main,
            "toggle",
            None,
            false,
        ));
    }
    for prompt in prompts {
        specs.push((
            format!("prompt:{}", prompt.id),
            prompt.title.clone(),
            &prompt.hotkey,
            &prompt.hotkey_mode,
            Some(prompt.id.clone()),
            false,
        ));
    }
    let mut candidates = Vec::new();
    let mut issues = Vec::new();
    for (id, title, text, mode, prompt_id, wake_only) in specs {
        if text.is_empty() {
            continue;
        }
        let Some(keys) = parse_hotkey_string(text) else {
            issues.push(BindingIssue {
                id,
                fingerprint: format!("invalid:{text}"),
                message: format!("{title}：无法识别快捷键 {text}"),
            });
            continue;
        };
        let mode = HotkeyMode::from_str(mode);
        if (wake_only || mode == HotkeyMode::Hold)
            && !keys.iter().any(|k| recorder::is_valid_main_first_key(*k))
        {
            issues.push(BindingIssue {
                id,
                fingerprint: format!("first-key:{keys:?}"),
                message: format!("{title}：快捷键需包含功能键、Tab 或修饰键"),
            });
            continue;
        }
        // A wake combination must land on a regular key: ⌃⌘, ⇧⌥ and friends are
        // system / IME chords that fire constantly during typing.
        if wake_only && keys.len() > 1 && keys.iter().all(|k| recorder::is_modifier_key(*k)) {
            issues.push(BindingIssue {
                id,
                fingerprint: format!("chord:{keys:?}"),
                message: format!("{title}：修饰键组合需包含一个常规按键（如 ⌃⌘L）"),
            });
            continue;
        }
        candidates.push((
            id,
            title,
            text.to_string(),
            HotkeyBinding {
                keys,
                mode,
                prompt_id,
                wake_only,
            },
        ));
    }
    let main_keys = candidates
        .iter()
        .find(|(_, _, _, b)| b.wake_only)
        .map(|(_, _, _, b)| b.keys.clone());
    let protected = |b: &HotkeyBinding| {
        b.wake_only
            || (b.mode == HotkeyMode::Toggle
                && b.prompt_id.is_none()
                && Some(&b.keys) == main_keys.as_ref())
    };
    let raw_toggle_stop =
        |b: &HotkeyBinding| b.mode == HotkeyMode::Toggle && b.prompt_id.is_none() && !b.wake_only;
    let mut disabled = HashSet::new();
    for i in 0..candidates.len() {
        for j in i + 1..candidates.len() {
            let (id_a, title_a, text_a, a) = &candidates[i];
            let (id_b, title_b, text_b, b) = &candidates[j];
            let same = a.keys == b.keys;
            // Only the raw toggle stop is allowed to share the global wake key.
            if same && a.wake_only && raw_toggle_stop(b) {
                continue;
            }
            let contains_hold = (a.mode == HotkeyMode::Hold || b.mode == HotkeyMode::Hold)
                && (a.keys.is_subset(&b.keys) || b.keys.is_subset(&a.keys));
            if same || contains_hold {
                let message = format!(
                    "{title_a}（{text_a}）与{title_b}（{text_b}）快捷键重复或包含，冲突绑定已停用"
                );
                if !protected(a) {
                    disabled.insert(i);
                    issues.push(BindingIssue {
                        id: id_a.clone(),
                        fingerprint: format!("conflict:{id_a}:{:?}:{id_b}:{:?}", a.keys, b.keys),
                        message: message.clone(),
                    });
                }
                if !protected(b) {
                    disabled.insert(j);
                    issues.push(BindingIssue {
                        id: id_b.clone(),
                        fingerprint: format!("conflict:{id_a}:{:?}:{id_b}:{:?}", a.keys, b.keys),
                        message,
                    });
                }
            }
        }
    }
    let bindings: Vec<HotkeyBinding> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, _, _, b))| (!disabled.contains(&i)).then_some(b))
        .collect();
    (bindings, issues)
}

/// A mode switch must clear an invalid target even if the same conflict was
/// already present in an older configuration. Unrelated legacy issues remain.
pub fn validate_target(
    app: &AppSettings,
    prompts: &[PromptItem],
    target: &str,
) -> Result<(), String> {
    if let Some(issue) = compile_bindings(app, prompts)
        .1
        .into_iter()
        .find(|issue| issue.id == target)
    {
        return Err(issue
            .message
            .replace("，冲突绑定已停用", "，请使用独立快捷键"));
    }
    Ok(())
}

/// Existing invalid bindings must not prevent unrelated settings from being saved.
pub fn validate_change(
    app: &AppSettings,
    prompts: &[PromptItem],
    previous: &AppSettings,
    previous_prompts: &[PromptItem],
) -> Result<(), String> {
    let (_, old) = compile_bindings(previous, previous_prompts);
    let (_, new) = compile_bindings(app, prompts);
    if let Some(issue) = new.iter().find(|issue| {
        !old.iter()
            .any(|old| old.id == issue.id && old.fingerprint == issue.fingerprint)
    }) {
        return Err(issue
            .message
            .replace("，冲突绑定已停用", "，请使用独立快捷键"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use keytap::Key;
    use std::collections::BTreeSet;
    fn prompt(key: &str, mode: &str) -> PromptItem {
        serde_json::from_value(
            serde_json::json!({"id":"p","title":"润色","hotkey":key,"hotkey_mode":mode}),
        )
        .unwrap()
    }
    #[test]
    fn raw_stop_may_share_global_but_prompt_may_not() {
        let c = AppConfig::default();
        assert!(compile_bindings(&c.app, &[]).1.is_empty());
        let (bindings, issues) =
            compile_bindings(&c.app, &[prompt(crate::config::DEFAULT_HOTKEY, "toggle")]);
        assert!(!issues.is_empty());
        assert!(bindings.iter().any(|b| b.wake_only));
        assert!(!bindings.iter().any(|b| b.prompt_id.is_some()));
    }

    /// The global main key must toggle by itself: clearing the optional raw stop
    /// key leaves a wake binding plus an alias stop binding with the same keys, so
    /// the second press ends the recording the first press started.
    #[test]
    fn main_key_is_a_self_contained_toggle_without_a_separate_stop_key() {
        for main in ["F13", "ControlLeft+MetaLeft+L"] {
            let mut c = AppConfig::default();
            c.app.hotkey = main.into();
            c.app.voice_hotkey = Some(String::new());
            let (bindings, issues) = compile_bindings(&c.app, &[]);
            assert!(issues.is_empty(), "{main}: {issues:?}");
            let keys = parse_hotkey_string(main).unwrap();
            let wake = bindings.iter().find(|b| b.wake_only).expect("wake binding");
            let stop = bindings
                .iter()
                .find(|b| !b.wake_only && b.mode == HotkeyMode::Toggle && b.prompt_id.is_none())
                .expect("main toggle stop binding");
            assert_eq!(wake.keys, keys);
            assert_eq!(stop.keys, keys, "the same chord stops what it started");
            assert_eq!(bindings.len(), 2);
        }
    }

    /// ⌃⌘ and ⇧⌥ are system / IME chords, so they are rejected as the global main
    /// key — with no wake binding, no stop alias is registered either.
    #[test]
    fn a_pure_modifier_chord_is_not_a_usable_main_key() {
        for chord in ["ControlLeft+MetaLeft", "ShiftLeft+AltRight"] {
            let mut c = AppConfig::default();
            c.app.hotkey = chord.into();
            c.app.voice_hotkey = Some(String::new());
            let (bindings, issues) = compile_bindings(&c.app, &[]);
            assert!(!issues.is_empty(), "{chord} must be reported");
            assert!(
                !bindings.iter().any(|b| b.wake_only),
                "{chord} must never wake the microphone"
            );
            assert!(
                !bindings
                    .iter()
                    .any(|b| !b.wake_only && b.mode == HotkeyMode::Toggle),
                "a main key that cannot wake must not register a stop alias"
            );
            let previous = AppConfig::default();
            assert!(
                validate_change(&c.app, &[], &previous.app, &[]).is_err(),
                "saving {chord} as the main key is a new conflict"
            );
        }
        // A single designated modifier stays valid — it is the product default.
        let mut single = AppConfig::default();
        single.app.hotkey = "MetaRight".into();
        single.app.voice_hotkey = Some(String::new());
        assert!(compile_bindings(&single.app, &[]).1.is_empty());
    }

    /// End-to-end contract for the compiled bindings: the exact list the app
    /// registers starts a recording on the first chord press and ends it on the
    /// second, with no separate stop key configured.
    #[test]
    fn the_compiled_main_chord_toggles_a_recording() {
        use super::super::matcher::{HotkeyAction, MatcherState};
        use keytap::EventKind;
        use std::time::{Duration, Instant};

        let mut c = AppConfig::default();
        c.app.hotkey = "ControlLeft+MetaLeft+L".into();
        c.app.voice_hotkey = Some(String::new());
        let (bindings, issues) = compile_bindings(&c.app, &[]);
        assert!(issues.is_empty());

        let chord = [Key::ControlLeft, Key::MetaLeft, Key::L];
        let mut matcher = MatcherState::new();
        let base = Instant::now();
        let mut cycle = |offset: u64| {
            let mut actions = Vec::new();
            for (i, key) in chord.into_iter().enumerate() {
                actions.extend(matcher.process_at(
                    EventKind::KeyDown(key),
                    &bindings,
                    base + Duration::from_millis(offset + i as u64),
                ));
            }
            for (i, key) in chord.into_iter().rev().enumerate() {
                actions.extend(matcher.process_at(
                    EventKind::KeyUp(key),
                    &bindings,
                    base + Duration::from_millis(offset + 10 + i as u64),
                ));
            }
            actions
        };

        assert_eq!(
            cycle(0),
            vec![HotkeyAction::StartRecording {
                is_main: true,
                mode: HotkeyMode::Toggle,
                prompt_id: None,
            }]
        );
        assert_eq!(
            cycle(30),
            vec![HotkeyAction::StopRecording { prompt_id: None }]
        );
        assert_eq!(
            cycle(60),
            vec![HotkeyAction::StartRecording {
                is_main: true,
                mode: HotkeyMode::Toggle,
                prompt_id: None,
            }]
        );
    }

    /// A stop key disabled by a conflict must not leave the wake key with no way
    /// to end a session: the main-key alias is protected and keeps the toggle.
    #[test]
    fn a_conflicting_stop_key_leaves_a_working_toggle() {
        let mut c = AppConfig::default();
        c.app.voice_hotkey = Some("ShiftLeft".into());
        let prompts = vec![prompt("ShiftLeft", "toggle")];
        let (bindings, issues) = compile_bindings(&c.app, &prompts);
        assert!(!issues.is_empty(), "the duplicate is reported");
        assert!(bindings.iter().any(|b| b.wake_only));
        assert!(
            bindings.iter().any(|b| !b.wake_only
                && b.mode == HotkeyMode::Toggle
                && b.prompt_id.is_none()
                && Some(&b.keys) == parse_hotkey_string(c.app.hotkey.as_str().unwrap()).as_ref()),
            "the main key keeps its protected stop alias"
        );
    }

    /// Modifier combinations are ordinary bindings: the whole chord is stored,
    /// and the optional independent stop key is registered next to the alias.
    #[test]
    fn modifier_combinations_register_exactly_the_configured_chord() {
        for chord in [
            "ControlLeft+MetaLeft+L",
            "AltLeft+ShiftLeft+D",
            "ControlLeft+AltLeft+MetaLeft+K",
        ] {
            let mut c = AppConfig::default();
            c.app.hotkey = chord.into();
            c.app.voice_hotkey = Some("ShiftRight".into());
            let (bindings, issues) = compile_bindings(&c.app, &[]);
            assert!(issues.is_empty(), "{chord}: {issues:?}");
            let expected = parse_hotkey_string(chord).unwrap();
            assert!(bindings.iter().any(|b| b.wake_only && b.keys == expected));
            assert!(bindings.iter().any(|b| !b.wake_only
                && b.mode == HotkeyMode::Toggle
                && b.keys == BTreeSet::from([Key::ShiftRight])));
            assert_eq!(
                bindings
                    .iter()
                    .filter(|b| b.keys == expected && !b.wake_only && b.mode == HotkeyMode::Toggle)
                    .count(),
                1,
                "exactly one stop alias, next to the independent stop key"
            );
            assert_eq!(bindings.len(), 3, "wake + alias + independent stop key");
        }
    }

    /// A separately configured stop key never replaces the toggle: the main key
    /// still ends the recording it started.
    #[test]
    fn the_main_key_always_stops_its_own_session_in_toggle_mode() {
        for (main, stop) in [
            ("F13", "ShiftRight"),
            ("ControlLeft+MetaLeft+L", "AltRight"),
        ] {
            let mut c = AppConfig::default();
            c.app.hotkey = main.into();
            c.app.voice_hotkey = Some(stop.into());
            let (bindings, issues) = compile_bindings(&c.app, &[]);
            assert!(issues.is_empty(), "{main}/{stop}: {issues:?}");
            let keys = parse_hotkey_string(main).unwrap();
            assert!(
                bindings
                    .iter()
                    .any(|b| !b.wake_only && b.mode == HotkeyMode::Toggle && b.keys == keys),
                "{main} must also be a stop binding"
            );
        }
    }

    /// When the stop key already is the main key (the shipped default), the alias
    /// exists once: a duplicated binding would report a self-conflict.
    #[test]
    fn an_alias_stop_key_is_never_registered_twice() {
        for main in [crate::config::DEFAULT_HOTKEY, "ControlLeft+MetaLeft+L"] {
            let mut c = AppConfig::default();
            c.app.hotkey = main.into();
            c.app.voice_hotkey = Some(main.into());
            let (bindings, issues) = compile_bindings(&c.app, &[]);
            assert!(issues.is_empty(), "{main}: {issues:?}");
            let keys = parse_hotkey_string(main).unwrap();
            assert_eq!(bindings.len(), 2, "wake + one stop alias for {main}");
            assert_eq!(
                bindings
                    .iter()
                    .filter(|b| b.keys == keys && !b.wake_only && b.mode == HotkeyMode::Toggle)
                    .count(),
                1
            );
        }
    }

    /// Hold mode must not hand the main key a stop binding: its sessions end on
    /// release of their own hold shortcut, so the wake key cannot start one.
    #[test]
    fn hold_mode_keeps_the_main_key_wake_only() {
        let mut c = AppConfig::default();
        c.app.hotkey = "ControlLeft+MetaLeft+L".into();
        c.app.hotkey_mode = "hold".into();
        c.app.voice_hotkey = Some(String::new());
        let (bindings, issues) = compile_bindings(&c.app, &[]);
        assert!(issues.is_empty());
        assert_eq!(bindings.len(), 1);
        assert!(bindings[0].wake_only);
        assert!(!bindings
            .iter()
            .any(|b| !b.wake_only && b.mode == HotkeyMode::Toggle));
    }

    /// A template that reuses the main chord is a real conflict; the global key
    /// keeps working and only the template binding is disabled.
    #[test]
    fn a_template_may_not_reuse_the_main_chord() {
        let mut c = AppConfig::default();
        c.app.hotkey = "ControlLeft+MetaLeft+L".into();
        c.app.voice_hotkey = Some(String::new());
        let (bindings, issues) =
            compile_bindings(&c.app, &[prompt("ControlLeft+MetaLeft+L", "toggle")]);
        assert!(!issues.is_empty());
        assert!(bindings.iter().all(|b| b.prompt_id.is_none()));
        assert_eq!(bindings.len(), 2);
        assert!(validate_target(
            &c.app,
            &[prompt("ControlLeft+MetaLeft+L", "toggle")],
            "prompt:p"
        )
        .is_err());
    }

    /// Recording the main chord and clearing the stop key is a valid save: the
    /// self-toggle alias must never be reported as a new conflict.
    #[test]
    fn saving_a_main_chord_with_a_cleared_stop_key_is_valid() {
        let previous = AppConfig::default();
        let mut changed = previous.clone();
        changed.app.hotkey = "ControlLeft+MetaLeft+L".into();
        changed.app.voice_hotkey = Some(String::new());
        assert!(validate_change(&changed.app, &[], &previous.app, &[]).is_ok());
        assert!(validate_target(&changed.app, &[], "main").is_ok());
        assert!(validate_target(&changed.app, &[], MAIN_TOGGLE_STOP_ID).is_ok());
    }
    #[test]
    fn hold_containment_is_checked_in_both_directions() {
        for (global, hold_key) in [
            ("ControlLeft", "ControlLeft+C"),
            ("ControlLeft+C", "ControlLeft"),
        ] {
            let mut c = AppConfig::default();
            c.app.hotkey = global.into();
            c.app.voice_hotkey = Some(global.into());
            let (bindings, issues) = compile_bindings(&c.app, &[prompt(hold_key, "hold")]);
            assert!(!issues.is_empty());
            assert!(bindings.iter().any(|b| b.wake_only));
            assert!(!bindings.iter().any(|b| b.mode == HotkeyMode::Hold));
        }
    }
    #[test]
    fn toggle_containment_is_allowed() {
        let mut c = AppConfig::default();
        c.app.hotkey = "ControlLeft".into();
        c.app.voice_hotkey = Some("ControlLeft".into());
        assert!(
            compile_bindings(&c.app, &[prompt("ControlLeft+ShiftLeft", "toggle")])
                .1
                .is_empty()
        );
    }
    #[test]
    fn plain_typing_hold_is_disabled_but_typing_toggle_stop_is_allowed() {
        let c = AppConfig::default();
        assert!(!compile_bindings(&c.app, &[prompt("C", "hold")])
            .1
            .is_empty());
        assert!(compile_bindings(&c.app, &[prompt("C", "toggle")])
            .1
            .is_empty());
    }
    #[test]
    fn existing_conflicts_do_not_block_unrelated_saves_but_new_ones_do() {
        let c = AppConfig::default();
        let p = vec![prompt(crate::config::DEFAULT_HOTKEY, "toggle")];
        assert!(validate_change(&c.app, &p, &c.app, &p).is_ok());
        assert!(validate_change(&c.app, &p, &c.app, &[]).is_err());
    }
    #[test]
    fn global_raw_stop_alias_is_preserved_when_a_legacy_hold_conflicts() {
        let mut c = AppConfig::default();
        c.app.hotkey = "ControlLeft".into();
        c.app.voice_hotkey = Some("ControlLeft".into());
        let (bindings, issues) = compile_bindings(&c.app, &[prompt("ControlLeft+C", "hold")]);
        assert!(!issues.is_empty());
        assert_eq!(bindings.len(), 2);
        assert!(bindings.iter().all(|b| b.prompt_id.is_none()));
    }
    #[test]
    fn renaming_an_existing_conflicting_prompt_is_not_a_new_conflict() {
        let c = AppConfig::default();
        let old = vec![prompt(crate::config::DEFAULT_HOTKEY, "toggle")];
        let mut new = old.clone();
        new[0].title = "新标题".into();
        assert!(validate_change(&c.app, &new, &c.app, &old).is_ok());
        new[0].hotkey = format!("{}+ShiftLeft", crate::config::DEFAULT_HOTKEY);
        new[0].hotkey_mode = "hold".into();
        assert!(validate_change(&c.app, &new, &c.app, &old).is_err());
    }
    #[test]
    fn hold_only_configuration_is_valid_without_a_hands_free_stop() {
        let previous = AppConfig::default();
        let mut changed = previous.clone();
        changed.app.hotkey_mode = "hold".into();
        changed.app.voice_hotkey = Some("F2".into());
        assert!(validate_change(&changed.app, &[], &previous.app, &[]).is_ok());
        let (bindings, issues) = compile_bindings(&changed.app, &[]);
        assert!(bindings.iter().any(|b| b.wake_only));
        assert!(!bindings
            .iter()
            .any(|b| !b.wake_only && b.mode == HotkeyMode::Toggle));
        assert!(issues.is_empty());
    }
    #[test]
    fn mode_switch_checks_the_target_even_if_legacy_conflict_is_unchanged() {
        let c = AppConfig::default();
        let mut prompts = vec![prompt(crate::config::DEFAULT_HOTKEY, "toggle")];
        assert!(validate_change(&c.app, &prompts, &c.app, &prompts).is_ok());
        assert!(validate_target(&c.app, &prompts, "prompt:p").is_err());
        prompts[0].hotkey.clear();
        assert!(validate_target(&c.app, &prompts, "prompt:p").is_ok());
    }
    #[test]
    fn clearing_a_hold_key_is_a_valid_disabled_state_without_a_warning() {
        let mut c = AppConfig::default();
        c.app.hotkey_mode = "hold".into();
        c.app.voice_hotkey = Some(String::new());
        assert!(compile_bindings(&c.app, &[]).1.is_empty());
        assert!(validate_target(&c.app, &[], "voice").is_ok());
    }
}
