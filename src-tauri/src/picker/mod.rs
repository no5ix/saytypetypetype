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
//! `recording::paste_text` (clipboard + paste keystroke) followed by
//! `recording::finish_delivered_text` (history, cleanup, end cue), which records
//! the original transcript and only pastes the selected string.

#[cfg(target_os = "macos")]
mod hover;
pub(crate) mod model;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
// Only the non-macOS placement path converts through these window-API units;
// macOS places the `NSWindow` directly in AppKit points.
#[cfg(not(target_os = "macos"))]
use tauri::{LogicalPosition, LogicalSize, Position, Size};

#[cfg(target_os = "macos")]
use objc2::msg_send;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSPoint, NSRect, NSSize};

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
            // The workflow falls back to the immediate paste path, which still
            // supports ESC cancellation while the state is Finishing.
            hotkey::set_escape_enabled(&hc, true);
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

/// Paste first, release keyboard ownership second.
///
/// The release must never run while the paste is still in flight (user keys
/// would race the simulated paste) and must never run when the paste failed
/// (the picker stays open so the user can retry). Extracted so the ordering
/// invariant is explicit and unit-tested rather than implied by call order.
async fn paste_then_release<F, R>(paste: F, release: R) -> Result<(), String>
where
    F: std::future::Future<Output = Result<(), String>>,
    R: FnOnce(),
{
    let result = paste.await;
    if result.is_ok() {
        release();
    }
    result
}

/// Select an option. Safe to call from a shortcut and a click at the same time:
/// the store reserves the option so delivery happens at most once.
///
/// The paste phase is the only awaited part before keyboard ownership is handed
/// back; history, state reset, volume settle, and the closing cue run in the
/// background afterwards.
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
    // The paste is the only step that must complete before keyboard ownership
    // can be released; everything else is cleanup. The helper keeps that
    // ordering invariant explicit (and unit-tested).
    let result = paste_then_release(
        crate::recording::paste_text(&app, &selection.text, config.as_ref()),
        || {
            inner.picker.finish_delivery(session_id, true, None);
            // The paste has landed: hand the keyboard back immediately. Picker
            // key routing, the Escape/Enter gates it armed, and the recording
            // matcher are all released here — before any closing animation,
            // volume wait, history write, or sound. The user can type (or press
            // Enter to send) in the target app from this point on.
            if let Some(hc) = app.try_state::<HotkeyConfig>() {
                hotkey::clear_picker_bindings(&hc);
                hotkey::reset_recording(&hc);
            }
            finish_close(&app, close_ticket);
        },
    )
    .await;

    match result {
        Ok(()) => {
            // History/stats, the Idle state, overlay dismissal, the bounded
            // volume-settle wait, and the closing cue continue independently in
            // the background; none of them may block input.
            let task_app = app.clone();
            let task_inner = Arc::clone(&inner);
            let history_text = selection.history_text.clone();
            let task_polish = polish.clone();
            let volume_session = selection.volume_session;
            tauri::async_runtime::spawn(async move {
                crate::recording::finish_delivered_text(
                    &task_app,
                    &task_inner,
                    &history_text,
                    config.as_ref(),
                    task_polish,
                    volume_session,
                    volume_session,
                )
                .await;
            });
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

    // The recording workflow is over and its volume was restored when capture
    // stopped; this session-scoped call is an idempotent backstop that also
    // covers a picker opened without a stop-time restore.
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

/// Tracks in-flight native geometry applications. `show_window` claims a ticket
/// before anything else; the placement closure checks its ticket before touching
/// the window, so a stale async placement can never move, resize, or show a
/// newer session's picker.
#[derive(Debug)]
struct GeometryGuard {
    ticket: AtomicU64,
}

impl GeometryGuard {
    fn begin(&self) -> u64 {
        self.ticket.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, ticket: u64) -> bool {
        self.ticket.load(Ordering::SeqCst) == ticket
    }
}

static GEOMETRY: GeometryGuard = GeometryGuard {
    ticket: AtomicU64::new(0),
};

/// Whether an applied frame still differs from the request. 1.5pt absorbs
/// sub-point rounding; anything larger is a real constraint or scale drift and
/// is corrected by the retry below.
#[cfg(target_os = "macos")]
fn frame_drift(rect: &NSRect, x: f64, y: f64, width: f64, height: f64) -> bool {
    (rect.origin.x - x).abs() > 1.5
        || (rect.origin.y - y).abs() > 1.5
        || (rect.size.width - width).abs() > 1.5
        || (rect.size.height - height).abs() > 1.5
}

/// One session's placement request: panel size and origin (x/y in the native
/// bottom-left space on macOS, top-left logical elsewhere), plus the geometry
/// ticket and screen name carried for the diagnostics and staleness guard.
struct Placement {
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    gen: u64,
    screen: String,
}

/// Deterministically place the picker on its target screen and show it.
///
/// All of it happens inside one main-thread task against the live `NSWindow`:
/// `setFrame:` (bottom-left AppKit points, straight from the screen's raw
/// `visibleFrame`), immediate read-back, one bounded retry if AppKit
/// constrained the frame, and only then `orderFrontRegardless`.
///
/// The tao window APIs must not be used for this step: wry routes
/// `set_size`/`set_position` through the main GCD queue while `show` travels
/// via the event-loop proxy — two queues with no defined order between them.
/// Reproduced on a real three-display Mac: a reused picker window came to the
/// front with the *previous* session's size and position (the first session
/// showed at the builder default), and the queued correction landed after the
/// window was already visible. Direct `setFrame:` verified to converge before
/// the window is ordered in, across Retina↔1× transitions, vertically offset
/// displays, and hide-between-sessions reuse.
#[cfg(target_os = "macos")]
fn place_and_show(
    window: &tauri::WebviewWindow,
    placement: Placement,
    tx: tokio::sync::oneshot::Sender<bool>,
) -> Result<(), String> {
    let app = window.app_handle().clone();
    window.with_webview(move |pw| unsafe {
        let Placement {
            width,
            height,
            x,
            y,
            gen,
            screen,
        } = placement;
        let win: *mut AnyObject = pw.ns_window().cast();
        if win.is_null() {
            log_rec!(warn, "Picker geometry gen={gen}: NSWindow unavailable");
            let _ = tx.send(false);
            return;
        }
        if !GEOMETRY.is_current(gen) {
            // A newer session owns the window and will place it itself.
            let _ = tx.send(true);
            return;
        }
        let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
        let _: () = msg_send![win, setFrame: rect, display: true];
        let mut got: NSRect = msg_send![win, frame];
        let mut retries = 0u8;
        if frame_drift(&got, x, y, width, height) {
            retries = 1;
            let _: () = msg_send![win, setFrame: rect, display: true];
            got = msg_send![win, frame];
        }
        let converged = !frame_drift(&got, x, y, width, height);
        let scale: f64 = msg_send![win, backingScaleFactor];
        if converged {
            log_rec!(
                info,
                "Picker geometry gen={gen}: screen={screen} requested {:.0}x{:.0}@{:.0},{:.0} actual {:.0}x{:.0}@{:.0},{:.0} (bl points) scale={:.1} retries={}",
                width,
                height,
                x,
                y,
                got.size.width,
                got.size.height,
                got.origin.x,
                got.origin.y,
                scale,
                retries,
            );
        } else {
            log_rec!(
                error,
                "Picker geometry gen={gen} NOT converged: requested {:.0}x{:.0}@{:.0},{:.0} got {:.0}x{:.0}@{:.0},{:.0} (bl points, scale {:.1})",
                width,
                height,
                x,
                y,
                got.size.width,
                got.size.height,
                got.origin.x,
                got.origin.y,
                scale,
            );
        }
        // The geometry above is final before the window can appear; the
        // viewport follows because the web view is laid out at these bounds.
        // Mouse-moved delivery: the window stays non-key so the app must not
        // activate — accept our own synthetic moves when active too (they flow
        // natively to a non-key window only with this flag).
        let _: () = msg_send![win, setAcceptsMouseMovedEvents: true];
        // The bridge observes moves while the app is inactive; it replays them
        // into this window from its own handler.
        hover::enable(&app);
        let _: () = msg_send![win, orderFrontRegardless];
        let _ = tx.send(true);
    })
    .map_err(|error| error.to_string())
}

/// Non-macOS fallback: keep the plain window-API sequence. The queue race
/// documented on [`place_and_show`] is specific to tao/wry on macOS.
#[cfg(not(target_os = "macos"))]
fn place_and_show(
    window: &tauri::WebviewWindow,
    placement: Placement,
    tx: tokio::sync::oneshot::Sender<bool>,
) -> Result<(), String> {
    let _ = window.set_size(Size::Logical(LogicalSize::new(
        placement.width,
        placement.height,
    )));
    let _ = window.set_position(Position::Logical(LogicalPosition::new(
        placement.x,
        placement.y,
    )));
    let _ = window.show();
    let _ = tx.send(true);
    Ok(())
}

/// Create the picker window lazily, or reuse it if it already exists.
fn get_or_create_window(
    handle: &AppHandle,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
) -> Option<tauri::WebviewWindow> {
    if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
        return Some(window);
    }
    let builder =
        WebviewWindowBuilder::new(handle, PICKER_LABEL, WebviewUrl::App("picker.html".into()))
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
            .accept_first_mouse(true)
            // Start the first build near the intended geometry: the window API
            // default would otherwise be the very first frame the panel shows in.
            .inner_size(width, height)
            .position(x, y);
    match builder.build() {
        Ok(window) => {
            let _ = window.set_visible_on_all_workspaces(true);
            Some(window)
        }
        Err(error) => {
            log_rec!(error, "Failed to create picker window: {}", error);
            None
        }
    }
}

/// Show the picker for a new session: resolve layout against the target
/// display's work area, then place the native window deterministically before
/// it becomes visible. Reports success so the caller can fall back to
/// automatic paste if the renderer is unavailable.
async fn show_window(app: &AppHandle, target: &Option<TargetDisplay>, option_count: usize) -> bool {
    // Claim the window before anything else: an exit animation from the previous
    // session may still be waiting to hide it, and that hide must not fire after
    // this session has shown the window again.
    EXIT.invalidate();
    let gen = GEOMETRY.begin();
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
        let (bx, by) =
            display::bottom_center_position_bl(display.work_area_bl, layout.width, SCREEN_MARGIN);
        log_rec!(
            info,
            "Picker layout gen={gen}: screen={:?} frame={:.0}x{:.0}@{:.0},{:.0} work={:.0}x{:.0}@{:.0},{:.0} scale={:.1}, panel={:.0}x{:.0}@{:.0},{:.0}, cards={} cols={} rows={}",
            display.name,
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

        let window = match get_or_create_window(&handle, layout.width, layout.height, x, y) {
            Some(window) => window,
            None => {
                let _ = tx.send(false);
                return;
            }
        };
        if let Err(error) = place_and_show(
            &window,
            Placement {
                width: layout.width,
                height: layout.height,
                x: bx,
                y: by,
                gen,
                screen: display.name.clone(),
            },
            tx,
        ) {
            // `tx` was consumed by the failed enqueue, so the caller observes a
            // cancelled channel and takes the documented fallback path.
            log_rec!(warn, "Picker geometry gen={gen}: placement unavailable ({error})");
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
            log_rec!(warn, "Picker window placement timed out; falling back");
            destroy_window(app);
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
            // Kill the hover bridge before hiding, against the same ticket:
            // observed moves after this point belong to other apps only.
            hover::disable();
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
        hover::disable();
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
        hover::disable();
        if let Some(window) = handle.get_webview_window(PICKER_LABEL) {
            let _ = window.destroy();
        }
    });
}

fn emit_state(app: &AppHandle, view: &PickerView) {
    let _ = app.emit_to(PICKER_LABEL, "picker:state", view);
}

/// Diagnostic: the renderer reports the viewport it actually measures plus the
/// grid it applied. Logged next to the native placement line, one session tells
/// the whole story: what the work area asked for, what `NSWindow` ended up at,
/// and what the page really sees — including any column divergence.
/// (Exposed to the renderer through `commands::report_picker_layout`.)
pub(crate) fn report_picker_layout(
    session_id: u64,
    viewport_width: f64,
    viewport_height: f64,
    scale_factor: f64,
    desired_columns: usize,
    columns: usize,
) {
    if columns == desired_columns {
        log_rec!(
            info,
            "Picker viewport: session {session_id} viewport {:.0}x{:.0} dpr={:.1} columns={columns}",
            viewport_width,
            viewport_height,
            scale_factor,
        );
    } else {
        log_rec!(
            warn,
            "Picker viewport columns adjusted: session {session_id} viewport {:.0}x{:.0} dpr={:.1} columns={columns} (native desired {desired_columns})",
            viewport_width,
            viewport_height,
            scale_factor,
        );
    }
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

    /// Keyboard-ownership invariant: release happens only after the paste
    /// future resolves successfully. A pending paste keeps the picker modal, and
    /// a failed paste keeps it open for a retry.
    #[tokio::test]
    async fn keyboard_ownership_is_released_only_after_a_successful_paste() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let released = Arc::new(AtomicBool::new(false));
        let (paste_tx, paste_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        let flag = Arc::clone(&released);
        let delivery = tokio::spawn(async move {
            paste_then_release(async move { paste_rx.await.unwrap() }, move || {
                flag.store(true, Ordering::SeqCst)
            })
            .await
        });

        tokio::task::yield_now().await;
        assert!(
            !released.load(Ordering::SeqCst),
            "the keyboard must stay owned while the paste is in flight"
        );
        paste_tx.send(Ok(())).unwrap();
        assert!(delivery.await.unwrap().is_ok());
        assert!(
            released.load(Ordering::SeqCst),
            "a delivered paste must release the keyboard immediately"
        );

        // A failed paste must not release: the picker re-arms for a retry.
        let released_on_failure = AtomicBool::new(false);
        let failed = paste_then_release(async { Err("粘贴失败".to_string()) }, || {
            released_on_failure.store(true, Ordering::SeqCst);
        })
        .await;
        assert!(failed.is_err());
        assert!(
            !released_on_failure.load(Ordering::SeqCst),
            "a failed delivery must keep keyboard ownership with the picker"
        );
    }
}
