//! Tauri/AppKit layer for the multi-version transcription picker.
//!
//! The pure session state (ordering, result association, stale-session
//! rejection, at-most-once delivery) lives in [`model`]. This module owns the
//! parts that need a running app: creating/reusing the floating window, running
//! every configured template concurrently against the original transcript,
//! routing shortcut and mouse selection, and delivering the chosen text through
//! the existing paste pipeline.
//!
//! The picker never persists processed text: delivery goes through
//! `recording::deliver_text_result`, which records the original transcript in
//! history and only pastes the selected string.

pub(crate) mod model;

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::{LogicalPosition, LogicalSize, Position, Size};

use crate::app_state::{self, AppInner};
use crate::config::{AppConfig, PromptItem};
use crate::display::{self, TargetDisplay};
use crate::hotkey::{self, HotkeyConfig, PickerKeyBinding};
use crate::model::ModelRegistry;
use crate::overlay;

use model::{prompt_option_id, PickerOptionSpec, PickerResult, PickerView, ORIGINAL_OPTION_ID};

const PICKER_LABEL: &str = "picker";
const SCREEN_MARGIN: f64 = 24.0;

/// Open a picker session for `original` and start one request per configured
/// template. Returns `false` only when the window/renderer could not be
/// created, in which case the caller has already fallen back to normal paste.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open_session(
    app: &AppHandle,
    inner: &Arc<AppInner>,
    original: String,
    prompts: Vec<PromptItem>,
    config: &AppConfig,
    registry: &ModelRegistry,
    hotwords: Vec<String>,
    volume_session: u64,
) -> bool {
    // Drop any previous session (its late results are rejected by generation).
    let _ = inner.picker.take();

    let target = inner.session_target.lock().await.clone();
    let specs: Vec<PickerOptionSpec> = prompts
        .iter()
        .enumerate()
        .map(|(index, prompt)| PickerOptionSpec {
            id: prompt_option_id(&prompt.id),
            title: if prompt.title.trim().is_empty() {
                format!("模板 {}", index + 1)
            } else {
                prompt.title.clone()
            },
            hotkey: hotkey::format_hotkey_label(&prompt.hotkey),
            prompt_id: prompt.id.clone(),
        })
        .collect();

    let original_hotkey = hotkey::format_hotkey_label(config.app.hotkey.as_str().unwrap_or(""));
    let view = inner.picker.open(
        original.clone(),
        specs,
        original_hotkey,
        config.app.theme.clone(),
        target.clone(),
        volume_session,
    );

    // Route the same configured chords (main hotkey for "original", each
    // template's shortcut) to selection, filtered by the existing conflict
    // rules so disabled bindings stay disabled.
    let picker_bindings = picker_key_bindings(&config.app, &prompts);
    if let Some(hc) = app.try_state::<HotkeyConfig>() {
        hotkey::set_picker_bindings(&hc, view.session_id, picker_bindings);
    }

    // Create/show the renderer before starting work. If the window cannot be
    // created, fall back to the normal single-result path instead of stranding
    // the transcript with no way to select it.
    if !show_window(app, &target, view.options.len()).await {
        log_rec!(
            warn,
            "Picker window unavailable; falling back to automatic paste"
        );
        let _ = inner.picker.take();
        if let Some(hc) = app.try_state::<HotkeyConfig>() {
            hotkey::clear_picker_bindings(&hc);
        }
        return false;
    }
    emit_state(app, &view);
    // The indicator has done its job; the picker replaces it.
    overlay::hide(app);

    // Each template independently processes the ORIGINAL transcript. Results
    // are keyed by template id, so completion order never matters.
    let registry = registry.clone();
    let session_id = view.session_id;
    for spec in prompts
        .iter()
        .map(|prompt| (prompt_option_id(&prompt.id), prompt.id.clone()))
    {
        let app = app.clone();
        let inner = Arc::clone(inner);
        let config = config.clone();
        let prompts = prompts.clone();
        let hotwords = hotwords.clone();
        let registry = registry.clone();
        let original = original.clone();
        let (option_id, prompt_id) = spec;
        tauri::async_runtime::spawn(async move {
            let outcome = crate::llm::polish_transcript(
                &config,
                &prompts,
                Some(prompt_id.as_str()),
                &hotwords,
                &registry,
                &original,
            )
            .await;
            let result = match outcome {
                crate::llm::PolishOutcome::Polished(text) => PickerResult::Ready(text),
                crate::llm::PolishOutcome::NotPolished => {
                    PickerResult::Failed("该模板未配置提示词".to_string())
                }
                crate::llm::PolishOutcome::Failed(error) => {
                    PickerResult::Failed(error.user_message())
                }
            };
            if inner.picker.set_result(session_id, &option_id, result) {
                if let Some(view) = inner.picker.view(session_id) {
                    emit_state(&app, &view);
                }
            }
        });
    }

    log_rec!(
        info,
        "Multi-version picker opened ({} options, session {})",
        view.options.len(),
        view.session_id
    );
    true
}

/// Select an option. Safe to call from a shortcut and a click at the same time:
/// the store reserves the option so delivery happens at most once.
pub(crate) async fn select(app: AppHandle, session_id: u64, option_id: String) {
    let inner = Arc::clone(&app.state::<Arc<AppInner>>());
    let Some(selection) = inner.picker.begin_delivery(session_id, &option_id) else {
        return;
    };

    log_rec!(
        info,
        "Picker selection: {} ({} chars, session {})",
        selection.option_id,
        selection.text.chars().count(),
        selection.session_id
    );

    // Reflect the in-flight delivery in the UI; the window stays visible while
    // the paste runs so a failure can keep the text on screen.
    if let Some(view) = inner.picker.view(session_id) {
        emit_state(&app, &view);
    }

    // A mouse click activates VoicePaste on macOS. Put the captured target app
    // back in front before the paste keystroke, otherwise the selected text
    // would be inserted into the picker (or nowhere).
    if let Some(pid) = selection
        .target
        .as_ref()
        .and_then(|target| target.target.as_ref())
        .map(|target| target.pid)
    {
        if display::activate_target_app(pid) {
            tokio::time::sleep(Duration::from_millis(90)).await;
        } else {
            log_rec!(
                warn,
                "Could not re-activate target app pid {}; pasting",
                pid
            );
        }
    }

    let config = inner.config_manager.load_config().ok();
    let polish = crate::stats::PolishRecord {
        prompt_id: selection.prompt_id.clone(),
        raw_text: None,
    };
    let result = crate::recording::deliver_text_result(
        &app,
        &inner,
        &selection.text,
        &selection.history_text,
        config.as_ref(),
        polish,
        selection.volume_session,
    )
    .await;

    match result {
        Ok(()) => {
            inner.picker.finish_delivery(session_id, true, None);
            if let Some(hc) = app.try_state::<HotkeyConfig>() {
                hotkey::clear_picker_bindings(&hc);
                hotkey::reset_recording(&hc);
            }
            hide_window(&app);
            inner
                .volume_guard
                .restore_session(selection.volume_session)
                .await;
        }
        Err(message) => {
            log_rec!(warn, "Picker delivery failed: {}", message);
            // Keep the text and the picker: re-arm selection so the user can
            // retry (or choose another version) instead of losing the session.
            inner
                .picker
                .finish_delivery(session_id, false, Some(message));
            if let Some(hc) = app.try_state::<HotkeyConfig>() {
                let bindings = inner
                    .config_manager
                    .load_config()
                    .map(|config| {
                        picker_key_bindings(&config.app, &inner.config_manager.load_prompts())
                    })
                    .unwrap_or_default();
                hotkey::set_picker_bindings(&hc, session_id, bindings);
            }
            if let Some(view) = inner.picker.view(session_id) {
                emit_state(&app, &view);
            }
        }
    }
}

/// Dismiss the picker without pasting (ESC / explicit close).
pub(crate) async fn cancel(app: AppHandle, session_id: Option<u64>) {
    let inner = Arc::clone(&app.state::<Arc<AppInner>>());
    let Some(id) = session_id.or_else(|| inner.picker.current_id()) else {
        return;
    };
    let volume_session = inner.picker.volume_session(id);
    if !inner.picker.cancel(id) {
        // Delivery already won the race; let it finish.
        return;
    }
    log_rec!(info, "Multi-version picker dismissed (session {})", id);

    if let Some(hc) = app.try_state::<HotkeyConfig>() {
        hotkey::clear_picker_bindings(&hc);
        hotkey::reset_recording(&hc);
    }
    hide_window(&app);

    // The recording workflow is over: restore the recording session's volume,
    // leave the overlay hidden and settle back to idle without writing history
    // or pasting anything.
    if let Some(volume_session) = volume_session {
        inner.volume_guard.restore_session(volume_session).await;
    } else {
        inner.volume_guard.restore().await;
    }
    overlay::hide(&app);
    let _ = app.emit("overlay:event", serde_json::json!({ "type": "reset" }));
    app_state::set_app_state(&app, &inner, app_state::AppState::Idle).await;
}

/// Current view for the picker WebView (initial fetch and reconnection).
pub(crate) fn current_view(inner: &AppInner) -> Option<PickerView> {
    inner
        .picker
        .current_id()
        .and_then(|id| inner.picker.view(id))
}

// ---------------------------------------------------------------------------
// Shortcut wiring
// ---------------------------------------------------------------------------

/// Map the existing compiled bindings onto picker options. The main/voice keys
/// choose "original"; each prompt uses its own configured shortcut. Deduped by
/// (option, keys) so the wake/stop alias pair does not fire twice.
fn picker_key_bindings(
    app: &crate::config::AppSettings,
    prompts: &[PromptItem],
) -> Vec<PickerKeyBinding> {
    let (bindings, _) = hotkey::compile_bindings(app, prompts);
    let mut result: Vec<PickerKeyBinding> = Vec::new();
    for binding in bindings {
        let option_id = match binding.prompt_id.as_deref() {
            None => ORIGINAL_OPTION_ID.to_string(),
            Some(id) => prompt_option_id(id),
        };
        if result
            .iter()
            .any(|existing| existing.option_id == option_id && existing.keys == binding.keys)
        {
            continue;
        }
        result.push(PickerKeyBinding {
            option_id,
            keys: binding.keys,
        });
    }
    result
}

// ---------------------------------------------------------------------------
// Window management
// ---------------------------------------------------------------------------

/// Create the picker window lazily, or reuse it if it already exists. Runs on
/// the main thread because webview creation requires it, and reports success so
/// the caller can fall back to automatic paste if the renderer is unavailable.
async fn show_window(app: &AppHandle, target: &Option<TargetDisplay>, option_count: usize) -> bool {
    let app = app.clone();
    let handle = app.clone();
    let target = target.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let scheduled = app.run_on_main_thread(move || {
        let display = target
            .as_ref()
            .map(|target| target.display.clone())
            .unwrap_or_else(display::fallback_display);
        let (width, height) = display::picker_window_size(display.work_area, option_count);
        let (x, y) =
            display::bottom_center_position(display.work_area, width, height, SCREEN_MARGIN);

        if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
            let _ = window.set_size(Size::Logical(LogicalSize::new(width, height)));
            let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
            let _ = window.show();
            let _ = tx.send(true);
            return;
        }

        let builder =
            WebviewWindowBuilder::new(&handle, PICKER_LABEL, WebviewUrl::App("picker.html".into()))
                .title("VoicePaste")
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                // Never become the key window: the global keytap still routes the
                // shortcuts and the target application keeps focus for delivery.
                .focusable(false)
                .accept_first_mouse(true);

        match builder.build() {
            Ok(window) => {
                let _ = window.set_size(Size::Logical(LogicalSize::new(width, height)));
                let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
                let _ = window.set_visible_on_all_workspaces(true);
                let _ = window.show();
                let _ = tx.send(true);
            }
            Err(error) => {
                log_rec!(error, "Failed to create picker window: {}", error);
                let _ = tx.send(false);
            }
        }
    });

    if scheduled.is_err() {
        return false;
    }
    // Never leave the workflow hanging behind a busy main thread: fall back to
    // the normal paste path and make sure a late window creation is undone so an
    // empty always-on-top panel cannot be stranded on screen.
    match tokio::time::timeout(Duration::from_secs(5), rx).await {
        Ok(Ok(opened)) => opened,
        _ => {
            log_rec!(warn, "Picker window creation timed out; falling back");
            destroy_window(&app);
            false
        }
    }
}

fn hide_window(app: &AppHandle) {
    let app = app.clone();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
            let _ = window.hide();
        }
    });
}

/// Tear the picker window down entirely (fallback cleanup). Queued on the main
/// thread after any pending creation, so it cannot be raced by a late build.
fn destroy_window(app: &AppHandle) {
    let app = app.clone();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
            let _ = window.destroy();
        }
    });
}

fn emit_state(app: &AppHandle, view: &PickerView) {
    let _ = app.emit_to(PICKER_LABEL, "picker:state", view);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn prompt(id: &str, hotkey: &str) -> PromptItem {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": id,
            "hotkey": hotkey,
            "hotkey_mode": "toggle"
        }))
        .unwrap()
    }

    #[test]
    fn original_uses_main_hotkey_and_templates_use_their_own() {
        let mut config = AppConfig::default();
        config.app.hotkey = "ShiftRight".into();
        config.app.hotkey_mode = "toggle".into();
        config.app.voice_hotkey = Some("ShiftRight".into());
        let prompts = vec![prompt("one", "AltRight"), prompt("two", "T")];
        let bindings = picker_key_bindings(&config.app, &prompts);
        let original: Vec<_> = bindings
            .iter()
            .filter(|binding| binding.option_id == ORIGINAL_OPTION_ID)
            .collect();
        assert_eq!(original.len(), 1, "wake/stop aliases must dedupe");
        assert!(original[0].keys.contains(&keytap::Key::ShiftRight));
        assert!(bindings.iter().any(|binding| {
            binding.option_id == prompt_option_id("one")
                && binding.keys.contains(&keytap::Key::AltRight)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.option_id == prompt_option_id("two") && binding.keys.contains(&keytap::Key::T)
        }));
    }

    #[test]
    fn unbound_template_has_no_shortcut_but_still_gets_an_option() {
        let config = AppConfig::default();
        let bindings = picker_key_bindings(&config.app, &[prompt("none", "")]);
        assert!(!bindings
            .iter()
            .any(|binding| binding.option_id == prompt_option_id("none")));
    }
}
