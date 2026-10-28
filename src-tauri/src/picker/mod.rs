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

use std::sync::atomic::{AtomicU64, Ordering};
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

/// How long the renderer's pop-out animation needs. The native window has to stay
/// on screen for that long, so the hide is scheduled this far ahead — counted
/// from the moment the close *animation begins* (the selection/cancel
/// interaction itself), never from when the paste finishes. Text delivery keeps
/// running with the window on screen while the panel animates away.
const EXIT_ANIMATION_MS: u64 = 140;

/// Coordinates the deferred hide with the exit animation.
///
/// Every hide takes a ticket and only performs the native `hide` once the
/// animation has run *and* nothing newer has claimed the window. Opening a new
/// session (or hiding again) invalidates an in-flight exit, so a rapid
/// open/close/open sequence can never be swallowed by a late hide.
#[derive(Debug)]
struct ExitGuard {
    ticket: AtomicU64,
}

impl ExitGuard {
    /// Claim the window for an exit animation and return its ticket.
    fn begin(&self) -> u64 {
        self.ticket.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Cancel a pending exit (a new session is taking over the window).
    fn invalidate(&self) {
        self.ticket.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether `ticket` still owns the pending exit.
    fn is_current(&self, ticket: u64) -> bool {
        self.ticket.load(Ordering::SeqCst) == ticket
    }
}

static EXIT: ExitGuard = ExitGuard {
    ticket: AtomicU64::new(0),
};

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

    // Re-detect the target display now, at open time. The snapshot captured at
    // recording start can be stale: the user may have moved the target window to
    // another monitor, or screens may have been plugged/unplugged or re-resolved
    // while dictation ran. Reusing it sized the picker against the old screen's
    // work area (tiny on a big display, oversized on a small one).
    let target = match display::try_resolve_target_display_on_main(app).await {
        Some(fresh) => {
            let mut session = inner.session_target.lock().await;
            if let Some(previous) = session.as_ref() {
                if previous.display.frame != fresh.display.frame {
                    log_rec!(
                        info,
                        "Target display moved from {:?} to {:?}; re-sizing picker",
                        previous.display.frame,
                        fresh.display.frame
                    );
                }
            }
            *session = Some(fresh.clone());
            Some(fresh)
        }
        // Capture failed: keep whatever the recording start resolved rather
        // than degrading to the synthetic fallback while a real target exists.
        None => inner.session_target.lock().await.clone(),
    };
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

    // Reflect the in-flight delivery in the UI.
    if let Some(view) = inner.picker.view(session_id) {
        emit_state(&app, &view);
    }

    // Begin the dismissal in the same interaction that accepted the selection:
    // the panel starts its existing pop-out now while the clipboard/paste work
    // runs with the (animating) window still on screen. The native hide lands
    // when the animation is over; `finish_close` below is only the backstop.
    let close_ticket = begin_close(&app);

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
            finish_close(&app, close_ticket);
            inner
                .volume_guard
                .restore_session(selection.volume_session)
                .await;
        }
        Err(message) => {
            log_rec!(warn, "Picker delivery failed: {}", message);
            // Keep the text and the picker: re-arm selection so the user can
            // retry (or choose another version) instead of losing the session.
            // The dismissal that started at selection time is revoked — the
            // pending hide is invalidated by `show_window`, and the renderer is
            // told to return the panel to its open state.
            inner
                .picker
                .finish_delivery(session_id, false, Some(message));
            let option_count = inner
                .picker
                .view(session_id)
                .map(|view| view.options.len())
                .unwrap_or(1);
            let shown = show_window(&app, &selection.target, option_count).await;
            if shown {
                signal_reopen(&app);
            } else {
                log_rec!(warn, "Could not re-show picker after failed delivery");
            }
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
    // The Escape interaction itself starts the pop-out — no paste work is
    // pending, so the single animation timer is the only hide needed.
    begin_close(&app);

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
///
/// Ordering matters across monitors: tao applies `set_size`/`set_position` by
/// dispatching `setContentSize:`/`setFrameTopLeftPoint:` onto the main dispatch
/// queue, while `show` would order the window front immediately. Showing first
/// would flash the *previous* monitor's geometry for a run-loop pass. So the
/// size and position are queued here, and a follow-up task on the same FIFO
/// queue performs the `show` and verifies the geometry the window actually has
/// (points = physical ÷ the window's current backing scale) against the target
/// work area. Any mismatch is logged — it is the field diagnostic for the
/// "picker wrong size after moving to another monitor" class of bugs.
async fn show_window(app: &AppHandle, target: &Option<TargetDisplay>, option_count: usize) -> bool {
    // Claim the window before anything else: an exit animation from the previous
    // session may still be waiting to hide it, and that hide must not fire after
    // this session has shown the window again.
    EXIT.invalidate();
    let app = app.clone();
    let handle = app.clone();
    let target = target.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let scheduled = app.run_on_main_thread(move || {
        let display = target
            .as_ref()
            .map(|target| target.display.clone())
            .unwrap_or_else(display::fallback_display);
        let layout = display::picker_layout(display.work_area, option_count);
        let (x, y) =
            display::bottom_center_position(display.work_area, layout.width, layout.height, SCREEN_MARGIN);
        log_rec!(
            info,
            "Picker layout: frame={}x{}@{},{}, work={}x{}@{},{}, scale={:.1}, panel={:.0}x{:.0} at {:.0},{:.0}, cards={} cols={} rows={}",
            display.frame.width,
            display.frame.height,
            display.frame.x,
            display.frame.y,
            display.work_area.width,
            display.work_area.height,
            display.work_area.x,
            display.work_area.y,
            display.scale_factor,
            layout.width,
            layout.height,
            x,
            y,
            option_count,
            layout.columns,
            layout.rows,
        );

        let window = if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
            let _ = window.set_size(Size::Logical(LogicalSize::new(layout.width, layout.height)));
            let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
            window
        } else {
            let builder = WebviewWindowBuilder::new(
                &handle,
                PICKER_LABEL,
                WebviewUrl::App("picker.html".into()),
            )
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
                    let _ =
                        window.set_size(Size::Logical(LogicalSize::new(layout.width, layout.height)));
                    let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
                    let _ = window.set_visible_on_all_workspaces(true);
                    window
                }
                Err(error) => {
                    log_rec!(error, "Failed to create picker window: {}", error);
                    let _ = tx.send(false);
                    return;
                }
            }
        };

        // Runs after the queued size/position blocks above (same main queue,
        // FIFO): show, then read back what the window really ended up with.
        let show_handle = handle.clone();
        if show_handle.run_on_main_thread(move || {
            let _ = window.show();
            let scale = window.scale_factor().unwrap_or(1.0).max(0.5);
            if let (Ok(size), Ok(position)) = (window.inner_size(), window.outer_position()) {
                let actual_w = f64::from(size.width) / scale;
                let actual_h = f64::from(size.height) / scale;
                let actual_x = f64::from(position.x) / scale;
                let actual_y = f64::from(position.y) / scale;
                let fits = actual_x >= display.work_area.x - 1.0
                    && actual_y >= display.work_area.y - 1.0
                    && actual_x + actual_w <= display.work_area.x + display.work_area.width + 1.0
                    && actual_y + actual_h <= display.work_area.y + display.work_area.height + 1.0;
                if !fits
                    || (actual_w - layout.width).abs() > 1.0
                    || (actual_h - layout.height).abs() > 1.0
                {
                    log_rec!(
                        warn,
                        "Picker geometry drifted: wanted {:.0}x{:.0} at {:.0},{:.0}, got {:.0}x{:.0} at {:.0},{:.0} (window scale {:.1}, work {}x{}@{},{})",
                        layout.width,
                        layout.height,
                        x,
                        y,
                        actual_w,
                        actual_h,
                        actual_x,
                        actual_y,
                        scale,
                        display.work_area.width,
                        display.work_area.height,
                        display.work_area.x,
                        display.work_area.y,
                    );
                }
            }
        })
        .is_err()
        {
            // Event loop is gone; nothing can place the window any more.
            log_rec!(warn, "Could not schedule picker show on the main thread");
        }
        let _ = tx.send(true);
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

/// Begin the dismissal immediately: announce `picker:closing` so the renderer
/// starts its existing 130ms pop-out, and schedule the native hide for when
/// that animation ends. This must run in the same interaction that accepts the
/// selection (or the cancel), not after the clipboard/paste work — waiting for
/// delivery first is what made dismissal feel slow.
///
/// Returns the exit ticket; pass it to [`finish_close`] once the real work is
/// done so the window is hidden even if some other path already consumed the
/// animation timer. Re-opening a session calls [`ExitGuard::invalidate`] (via
/// `show_window`), so a pending hide can never swallow a newer picker.
fn begin_close(app: &AppHandle) -> u64 {
    let ticket = EXIT.begin();
    let _ = app.emit_to(
        PICKER_LABEL,
        "picker:closing",
        serde_json::json!({ "generation": ticket, "closing": true }),
    );

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(EXIT_ANIMATION_MS)).await;
        // A newer session or a newer hide owns the window now.
        if !EXIT.is_current(ticket) {
            return;
        }
        let window_owner = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = window_owner.get_webview_window(PICKER_LABEL) {
                let _ = window.hide();
            }
        });
    });
    ticket
}

/// Complete a close started by [`begin_close`]: hide now when this exit still
/// owns the window. Safe (and cheap) after the animation timer already hid it;
/// a no-op once a newer session or hide took over.
fn finish_close(app: &AppHandle, ticket: u64) {
    if !EXIT.is_current(ticket) {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
            let _ = window.hide();
        }
    });
}

/// Tell the renderer the window is visible again (used when a failed delivery
/// re-arms the session): clears the closing state on the *same* session id so
/// the panel returns to open instead of fading out invisibly.
fn signal_reopen(app: &AppHandle) {
    let _ = app.emit_to(
        PICKER_LABEL,
        "picker:closing",
        serde_json::json!({ "closing": false }),
    );
}

/// Tear the picker window down entirely (fallback cleanup). Queued on the main
/// thread after any pending creation, so it cannot be raced by a late build.
fn destroy_window(app: &AppHandle) {
    // No animation to wait for, and a pending exit must not target a dead window.
    EXIT.invalidate();
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

    /// The deferred hide must be cancellable: otherwise a re-open that lands while
    /// the pop-out is still running would be hidden by the previous session.
    #[test]
    fn only_the_newest_exit_may_hide_the_window() {
        let guard = ExitGuard {
            ticket: AtomicU64::new(0),
        };

        let first = guard.begin();
        assert!(guard.is_current(first), "the exit is pending at first");

        guard.invalidate();
        assert!(
            !guard.is_current(first),
            "opening a new session swallows the pending hide"
        );

        let second = guard.begin();
        assert!(guard.is_current(second));
        let third = guard.begin();
        assert!(
            !guard.is_current(second),
            "a newer hide replaces the old one"
        );
        assert!(guard.is_current(third));
    }

    /// The full dismissal lifecycle in ticket terms: `begin_close` starts the
    /// animation timer at the selection instant; a session re-open that lands
    /// while that timer is still in flight must invalidate it (that is what
    /// `show_window` does first thing), so the obsolete close can never hide
    /// the freshly opened picker.
    #[test]
    fn reopening_during_the_exit_animation_invalidates_the_pending_close() {
        let guard = ExitGuard {
            ticket: AtomicU64::new(0),
        };

        // User selects a result: the exit is claimed and the timer starts.
        let closing = guard.begin();
        assert!(
            guard.is_current(closing),
            "the close is pending mid-animation"
        );

        // A new session takes the window before the animation completes.
        guard.invalidate();
        assert!(
            !guard.is_current(closing),
            "the stale timer must not hide the new session"
        );
    }

    /// The window must not linger visible for a perceptible amount of time.
    /// The timer counts from the moment the close animation begins (the
    /// selection/cancel interaction), not from when delivery finishes.
    #[test]
    fn exit_animation_stays_short() {
        assert!((120..=180).contains(&EXIT_ANIMATION_MS));
    }
}
