//! keytap listener thread: receives raw keyboard events and dispatches matching
//! hotkey actions via the Tauri async runtime.

use keytap::{EventKind, Key, RecvTimeoutError, Tap};
use std::time::{Duration, Instant};

use super::matcher::HotkeyAction;
use super::{HotkeyConfig, HotkeyManager};

/// Start the global hotkey listener thread.
///
/// Spawns a background thread that receives raw keyboard events from keytap
/// and dispatches matching hotkey events via the Tauri async runtime.
///
/// When accessibility/input-monitoring permission is not granted (macOS/Linux),
/// logs a warning and returns a manager *without* an active listener so the
/// app can still start. The user can grant permission later and restart.
pub(crate) fn start_hotkey_listener(
    config: HotkeyConfig,
    app_handle: tauri::AppHandle,
) -> Result<HotkeyManager, keytap::Error> {
    install_suppress_handler(&config);

    let tap = match build_tap() {
        Ok(tap) => tap,
        Err(keytap::Error::PermissionDenied) => {
            log_hotkey!(
                warn,
                "Accessibility permission not granted — global hotkeys disabled"
            );
            return Ok(HotkeyManager { _config: config });
        }
        Err(e) => {
            log_hotkey!(
                error,
                "keytap init failed: {:?} — global hotkeys disabled",
                e
            );
            return Ok(HotkeyManager { _config: config });
        }
    };

    let config_clone = config.clone();
    let handle_clone = app_handle.clone();

    std::thread::Builder::new()
        .name("voicepaste-hotkey".into())
        .spawn(move || {
            run_listener_loop(&tap, &config_clone, &handle_clone);
        })
        .expect("failed to spawn hotkey listener thread");

    config.write().unwrap().tap_active = true;

    Ok(HotkeyManager { _config: config })
}

/// Create the keytap in filtering mode so suppressed keys (decision-window
/// ESC/Enter) are swallowed instead of reaching the foreground app. Falls
/// back to listen-only when the stronger Accessibility grant is missing —
/// hotkeys keep working, only the swallowing degrades.
fn build_tap() -> Result<Tap, keytap::Error> {
    match Tap::builder().filter_events().build() {
        Ok(tap) => {
            log_hotkey!(
                info,
                "Filtering keytap active (decision keys are swallowed)"
            );
            Ok(tap)
        }
        Err(filter_error) => {
            log_hotkey!(
                warn,
                "Filtering keytap unavailable ({:?}) — falling back to listen-only; \
                 decision keys will also reach the foreground app",
                filter_error
            );
            Tap::new()
        }
    }
}

/// Register the keytap suppress predicate: the per-key decision of which
/// presses are commands for VoicePaste and must not reach the foreground app.
/// Runs synchronously inside the platform event callback, so it takes one
/// short read lock and nothing more.
///
/// The captured config handle stays valid across reconfigs (`reload_bindings`
/// updates the inner state in place), and re-registering on reinit simply
/// replaces the closure — both calls are idempotent.
fn install_suppress_handler(config: &HotkeyConfig) {
    let config = config.clone();
    keytap::set_suppress_handler(Some(std::sync::Arc::new(move |key: Key| {
        let Ok(cfg) = config.read() else {
            return false;
        };
        if cfg.recording {
            // While the UI recorder captures a combination, swallow every key
            // so neither the settings window nor the IME reacts to it — a
            // Shift+F recording would otherwise toggle the IME or open the
            // candidate list. keytap still sends captured events to its
            // channel before blocking foreground delivery. All platforms
            // record through that event stream. ESC passes through so the
            // settings window can also handle cancellation. macOS modifier
            // FlagsChanged events are not swallowed by keytap.
            return key != Key::Escape;
        }
        if let Some(picker) = cfg.picker.as_ref() {
            // The picker is modal for its own keys: option shortcuts, ESC and
            // Enter must not leak into the target application. Ordinary typing
            // passes through so a stuck picker can never lock the keyboard.
            if matches!(key, Key::Escape | Key::Enter | Key::NumpadEnter) {
                return true;
            }
            if picker
                .bindings
                .iter()
                .any(|binding| binding.keys.contains(&key))
            {
                return true;
            }
        }
        if !cfg.matcher.should_swallow(key, &cfg.bindings, &[]) {
            return false;
        }
        cfg.matcher
            .should_swallow(key, &cfg.bindings, &super::physical::modifier_states())
    })));
}

/// Try to start the keytap listener if it is not already running.
///
/// Returns `true` if the listener is now active (either it was already
/// running, or we successfully created it post-startup).  Returns `false`
/// if the tap still cannot be created (e.g. permission still missing).
pub fn ensure_hotkey_active(config: &HotkeyConfig, app_handle: &tauri::AppHandle) -> bool {
    install_suppress_handler(config);

    {
        let cfg = config.read().unwrap();
        if cfg.tap_active {
            return true;
        }
    }

    let tap = match build_tap() {
        Ok(tap) => tap,
        Err(e) => {
            log_hotkey!(warn, "Still cannot create keytap: {:?}", e);
            return false;
        }
    };

    let config_clone = config.clone();
    let handle_clone = app_handle.clone();

    std::thread::Builder::new()
        .name("voicepaste-hotkey".into())
        .spawn(move || {
            run_listener_loop(&tap, &config_clone, &handle_clone);
        })
        .expect("failed to spawn hotkey listener thread");

    config.write().unwrap().tap_active = true;
    log_hotkey!(info, "Hotkey listener started (post-startup reinit)");
    true
}

/// Main loop for the listener thread.
///
/// Forwards raw keytap events to the shared [`MatcherState`] and dispatches
/// the resulting [`HotkeyAction`]s on the async runtime. Escape cancellation
/// and the UI hotkey-recorder relay are handled here, independent of chord
/// matching.
fn run_listener_loop(tap: &Tap, config: &HotkeyConfig, app_handle: &tauri::AppHandle) {
    let mut escape_was_pressed = false;
    let mut enter_was_pressed = false;
    let mut last_event_at = Instant::now();
    let (actions_tx, mut actions_rx) =
        tokio::sync::mpsc::unbounded_channel::<(u64, HotkeyAction)>();
    let action_handle = app_handle.clone();
    let action_config = config.clone();
    // Serialize microphone startup and release, but never queue a new recording
    // behind ASR/LLM finalization. ESC remains independent of that work.
    tauri::async_runtime::spawn(async move {
        let mut finishing: Option<tauri::async_runtime::JoinHandle<()>> = None;
        while let Some((generation, action)) = actions_rx.recv().await {
            match action {
                HotkeyAction::StartRecording { is_main, mode } => {
                    if action_config.read().unwrap().matcher.generation() != generation {
                        continue;
                    }
                    if finishing
                        .as_ref()
                        .is_some_and(|task| !task.inner().is_finished())
                    {
                        action_config.write().unwrap().matcher.reset_recording();
                        continue;
                    }
                    crate::recording::on_recording_start(action_handle.clone(), is_main, mode)
                        .await;
                    // Internal resets also occur on retry and failed startup;
                    // only tear down an actual microphone recording here.
                    if action_config.read().unwrap().matcher.generation() != generation
                        && crate::recording::is_recording(&action_handle)
                    {
                        crate::recording::on_escape(action_handle.clone()).await;
                    }
                }
                HotkeyAction::StopRecording { prompt_id } => {
                    if action_config.read().unwrap().matcher.generation() != generation {
                        continue;
                    }
                    let handle = action_handle.clone();
                    finishing = Some(tauri::async_runtime::spawn(async move {
                        crate::recording::on_recording_stop(handle, prompt_id).await;
                    }));
                }
            }
        }
    });

    loop {
        let wait = config
            .read()
            .unwrap()
            .matcher
            .pending_deadline()
            .map(|at| {
                at.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(100))
            })
            .unwrap_or(Duration::from_millis(100));
        match tap.recv_timeout(wait) {
            Ok(event) => {
                last_event_at = Instant::now();
                let event_kind = event.kind;

                // Snapshot all per-event config fields under one read lock
                // (there is no await point between them), instead of three
                // independent reads. `bindings` is reused for both the
                // allowlist check and the matcher, so it is cloned once here.
                let (is_recording, record_tx, escape_enabled, enter_enabled, bindings) = {
                    let cfg = config.read().unwrap();
                    (
                        cfg.recording,
                        cfg.record_tx.clone(),
                        cfg.escape_enabled || cfg.matcher.recording_mode().is_some(),
                        cfg.enter_enabled,
                        cfg.bindings.clone(),
                    )
                };

                // While the UI hotkey recorder is capturing a combination,
                // forward every event to it via the relay channel instead of
                // matching bindings. This avoids a second WH_KEYBOARD_LL hook
                // (unreliable on Windows with two taps per process).
                if is_recording {
                    config.write().unwrap().matcher.cancel_pending();
                    // Record every received event before recorder rules or
                    // main-hotkey validation can reject it. Never log typing
                    // outside an active shortcut-recording session.
                    log_hotkey!(
                        info,
                        "recorder stage=input event={:?} event_age_ms={}",
                        event.kind,
                        event.time.elapsed().as_millis()
                    );
                    if let Some(tx) = record_tx {
                        if let Err(error) = tx.try_send(event) {
                            log_hotkey!(
                                warn,
                                "recorder stage=relay outcome=send_failed error={:?}",
                                error
                            );
                        }
                    } else {
                        log_hotkey!(warn, "recorder stage=relay outcome=missing_receiver");
                    }
                    continue;
                }

                // Also cancel a 500ms candidate before recording/ESC gating begins.
                if matches!(event_kind, EventKind::KeyDown(Key::Escape)) {
                    config.write().unwrap().matcher.cancel_pending();
                }
                // Escape cancellation (independent of chord matching).
                if matches!(event_kind, EventKind::KeyUp(Key::Escape)) {
                    escape_was_pressed = false;
                }
                if escape_enabled
                    && matches!(event_kind, EventKind::KeyDown(Key::Escape))
                    && !escape_was_pressed
                {
                    escape_was_pressed = true;
                    config.write().unwrap().matcher.reset_recording();
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::recording::on_escape(handle).await;
                    });
                }

                // Enter confirm (paste the raw text), gated the same way as ESC.
                // Handled before the allowlist because Enter is not part of any
                // registered binding. Auto-repeat is suppressed via the
                // was_pressed latch so holding Enter fires once.
                if matches!(event_kind, EventKind::KeyUp(Key::Enter | Key::NumpadEnter)) {
                    enter_was_pressed = false;
                }
                if enter_enabled
                    && matches!(
                        event_kind,
                        EventKind::KeyDown(Key::Enter | Key::NumpadEnter)
                    )
                    && !enter_was_pressed
                {
                    enter_was_pressed = true;
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::recording::on_enter(handle).await;
                    });
                }

                // While the picker is open it owns the keyboard: recording
                // shortcuts must not start/stop a session, and the matcher must
                // not accumulate state behind the modal.
                let picker_active = config.read().unwrap().picker.is_some();
                if picker_active {
                    let action = {
                        let mut cfg = config.write().unwrap();
                        cfg.matcher.cancel_pending();
                        super::process_picker_event(&mut cfg, event_kind)
                    };
                    if let Some((session_id, option_id)) = action {
                        let handle = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            crate::picker::select(handle, session_id, option_id).await;
                        });
                    }
                    continue;
                }

                // Track all ordinary keys, including ones outside every binding,
                // so an extra letter can invalidate an otherwise matching chord.
                let actions = {
                    let mut cfg = config.write().unwrap();
                    let actions = cfg.matcher.process_at(event_kind, &bindings, event.time);
                    (cfg.matcher.generation(), actions)
                };
                for action in actions.1 {
                    let _ = actions_tx.send((actions.0, action));
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut cfg = config.write().unwrap();
                if cfg.recording {
                    cfg.matcher.cancel_pending();
                    continue;
                }
                // Check before delayed activation as well as during idle recovery:
                // a lost early release must not start a microphone at the deadline.
                // Unknown native states remain untouched.
                let deadline_due = cfg
                    .matcher
                    .pending_deadline()
                    .is_some_and(|at| Instant::now() >= at);
                if deadline_due || last_event_at.elapsed() >= Duration::from_secs(2) {
                    let released: Vec<_> = cfg
                        .matcher
                        .held
                        .iter()
                        .copied()
                        .filter(|key| {
                            let pressed = super::physical::is_pressed(*key);
                            if deadline_due {
                                // Only a complete configured hold chord reaches this
                                // probe: never log ordinary typing outside a candidate.
                                log_hotkey!(
                                    info,
                                    "hold stage=deadline key={:?} native_pressed={:?}",
                                    key,
                                    pressed
                                );
                            }
                            pressed == Some(false)
                        })
                        .collect();
                    if cfg.matcher.recover_released(&released) {
                        cfg.matcher.reset_recording();
                        let handle = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            crate::recording::on_escape(handle).await;
                        });
                    }
                }
                for action in cfg.matcher.tick(Instant::now()) {
                    let _ = actions_tx.send((cfg.matcher.generation(), action));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                log_hotkey!(debug, "Tap disconnected, listener thread exiting");
                break;
            }
        }
    }
}

/// Reset the matcher's recording tracking. Called by lib.rs when a recording
/// is cancelled, or when a start is diverted to retry or fails, so the matcher
/// does not believe it is still recording.
pub fn reset_recording(config: &HotkeyConfig) {
    config.write().unwrap().matcher.reset_recording();
}
