//! Recording lifecycle: start, stop, cancel, and retry/failure abort paths.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::app_state::{self, set_app_state, ActivePromptId, RecordingState};
use crate::overlay;

use super::capture;
use super::cue;
use super::finalize;
use super::history;
use super::session;
use super::wav;
use super::{
    begin_output_volume_session, is_current_epoch, lower_output_volume_for,
    reset_matcher_recording, restore_output_volume_for,
};

/// Upper bound on waiting for the start cue before dimming. The bundled cue is
/// ~0.5 s; this only guards against a corrupt or unusually long custom file so
/// the volume workflow can never stall indefinitely.
const START_CUE_VOLUME_WAIT: Duration = Duration::from_millis(2500);

/// Upper bound awaiting the dim ramp before capture starts. The ramp glides
/// over `RAMP_DURATION` (400 ms) inside its volume task; twice the duration
/// plus join slack covers a slow main-thread hand-off, and a wedged volume
/// controller must not prevent dictation from starting at all.
const DIM_COMPLETION_BOUND: Duration = Duration::from_millis(1000);

/// Start recording from idle state. Used by both toggle and hold modes.
pub(super) async fn start_recording(app_handle: AppHandle) {
    let app_inner = app_handle.state::<Arc<app_state::AppInner>>();
    let recording_state = app_handle.state::<RecordingState>();

    // Mark as recording
    *recording_state.0.lock().unwrap() = true;
    *app_inner.stop_started_at.lock().unwrap() = None;

    // 1. Load config
    let config = match app_inner.config_manager.load_config() {
        Ok(c) => c,
        Err(e) => {
            *recording_state.0.lock().unwrap() = false;
            log_rec!(error, "Failed to load config: {}", e);
            overlay::show_restart_error(&app_handle, &app_inner, &format!("配置加载失败: {}", e))
                .await;
            return;
        }
    };

    // Prompt-specific LLM validation is deferred to finalize_and_paste: the
    // prompt is resolved from the stop chord (not known at start), and a bad
    // LLM config surfaces there with the provider's own reason plus a retry.

    let _ = app_handle.emit("overlay:event", serde_json::json!({
        "type": "appearance", "payload": { "theme": config.app.theme, "overlayStyle": config.app.overlay_style }
    }));

    // Settle any parked polish before recording again. Otherwise its auto-paste
    // timer would fire mid-recording and drop the previous transcript into
    // whatever now has focus. This also restores the previous session's volume
    // before the new session snapshots the current state.
    super::polish::deliver_raw(&app_handle, &app_inner, "superseded by a new recording").await;

    *app_inner.latest_transcript.lock().await = (String::new(), String::new());
    app_inner.recording_audio.lock().await.clear();
    *app_inner.current_recording_wav.lock().await = None;
    *app_inner.current_retry_of.lock().await = None;
    *app_inner.current_failure_ts.lock().await = None;
    let _ = app_handle.emit("overlay:event", serde_json::json!({ "type": "reset" }));

    // Resolve which display holds the application that will receive the text.
    // Captured once per session so the indicator (and later the picker) follow
    // the target app across displays instead of always using the primary one.
    let target = crate::display::resolve_target_display_on_main(&app_handle).await;
    *app_inner.session_target.lock().await = Some(target.clone());

    // Re-position before showing so the overlay follows the current display layout
    // and the target application.
    overlay::position_overlay_on(&app_handle, &target);
    overlay::show(&app_handle);

    // 2. Enter the Connecting baseline for the rest of startup. The microphone
    //    is deliberately NOT started yet: with music playing at dictation time,
    //    whatever the mic picks up while the output volume is still at the
    //    user's level would be streamed into the session and transcribed.
    //    Capture only starts after the start cue has played and the volume
    //    dim ramp has completed (step 7).
    set_app_state(&app_handle, &app_inner, app_state::AppState::Connecting).await;

    // 3. Active hotwords for this session (also reused on reconnect).
    let hotwords = app_inner.hotword_manager.active_words();
    log_rec!(
        debug,
        "Active hotwords ({}): {:?}",
        hotwords.len(),
        hotwords
    );

    // 4. Play the start cue and enter Recording back-to-back: the cue tells the user
    //    they may speak, so the session must accept audio the instant the cue
    //    ends. The ASR session connects in the background so the user can speak
    //    as soon as the (local, fast) mic is ready instead of waiting on the
    //    (remote, variable) network handshake; audio captured before the session
    //    is ready is buffered and flushed once it connects.
    let my_epoch = app_inner
        .session_epoch
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    app_inner.pending_audio.lock().await.clear();
    *app_inner.asr_session.lock().await = None;
    *app_inner.accumulated_text.lock().await = String::new();

    let (connect_tx, connect_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    *app_inner.connect_rx.lock().await = Some(connect_rx);

    // Claim the system-volume snapshot BEFORE the cue. The dim itself is
    // deferred until the cue has finished so the start sound plays at the
    // user's real volume; if the user stops during the cue, cleanup clears the
    // session and the deferred dim becomes a no-op instead of muting the
    // already-finished workflow.
    begin_output_volume_session(&app_inner, my_epoch).await;
    // ESC can land while the volume session is being claimed. A cancelled start
    // must release that claim itself, otherwise its deferred dim would run with
    // no terminal path left to restore the volume.
    if !*recording_state.0.lock().unwrap() {
        log_rec!(info, "Cancelled during volume session start; aborting");
        capture::stop_capture(&app_inner).await;
        restore_output_volume_for(&app_inner, my_epoch).await;
        overlay::hide(&app_handle);
        set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
        return;
    }
    // A newer session superseded this start; it owns the state and volume now.
    if !is_current_epoch(&app_inner, my_epoch) {
        return;
    }
    let start_cue = cue::emit_cue_tracked(&app_handle, &app_inner, "start");
    set_app_state(&app_handle, &app_inner, app_state::AppState::Recording).await;
    // The same cancel can land while the cue/state transition is applied, after
    // which no stop hotkey will finalize this session. Settle it cleanly.
    if !*recording_state.0.lock().unwrap() {
        log_rec!(info, "Cancelled during recording start; aborting");
        capture::stop_capture(&app_inner).await;
        restore_output_volume_for(&app_inner, my_epoch).await;
        overlay::hide(&app_handle);
        set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
        return;
    }
    if !is_current_epoch(&app_inner, my_epoch) {
        return;
    }

    overlay::begin_recording_hints(&app_handle, &app_inner).await;

    // 5. Connect the ASR session in the background; attach it once ready.
    let connect_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        session::connect_and_attach(connect_handle, config, hotwords, my_epoch, connect_tx).await;
    });

    // 6. Wait for the start cue to finish, then lower the output volume and
    //    only afterwards start the microphone: music playing at dictation time
    //    must not be streamed in while the output is still at the user's
    //    volume, so the dim ramp has to be *complete* before any capture
    //    happens. `dim_session` only schedules its glide on a detached thread,
    //    so the follow-up `wait_for_ramp` is what actually proves completion.
    //    Both steps are bounded so a wedged volume controller can never stall
    //    the session; on a failed or late dim, capture still starts because
    //    losing the recording is worse than louder background.
    if let Some(cue) = start_cue {
        cue.wait(START_CUE_VOLUME_WAIT).await;
    }
    let dim_to_completion = async {
        lower_output_volume_for(&app_inner, my_epoch).await;
        app_inner.volume_guard.wait_for_ramp().await;
    };
    if tokio::time::timeout(DIM_COMPLETION_BOUND, dim_to_completion)
        .await
        .is_err()
    {
        log_rec!(warn, "Volume dim deadline elapsed; starting capture anyway");
    }
    // The user can cancel while the cue plays or the volume glides down: a
    // cancelled start must not open a capture stream, and its volume claim has
    // to be released through the normal terminal path.
    if !*recording_state.0.lock().unwrap() {
        log_rec!(info, "Cancelled during volume dim; aborting start");
        restore_output_volume_for(&app_inner, my_epoch).await;
        overlay::hide(&app_handle);
        set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
        return;
    }
    // A newer session superseded this start; it owns the state and volume now.
    if !is_current_epoch(&app_inner, my_epoch) {
        return;
    }

    // 7. Warm up microphone capture (native cpal on macOS + Windows) — first
    //    audio the session ever records is therefore captured after the dim
    //    completed. The capture owns its own ready signal (a oneshot the input
    //    thread fires once the stream is built), so there is no renderer
    //    warmup round-trip to wait on.
    if let Err(e) = capture::start_capture(app_handle.clone(), Arc::clone(&app_inner)).await {
        *recording_state.0.lock().unwrap() = false;
        restore_output_volume_for(&app_inner, my_epoch).await;
        set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
        log_rec!(warn, "Native audio warmup failed: {}", e);
        reset_matcher_recording(&app_handle);
        overlay::show_restart_error(&app_handle, &app_inner, &e).await;
        return;
    }
    // Check if recording was cancelled during warmup (hold mode: quick press-release)
    if !*recording_state.0.lock().unwrap() {
        log_rec!(warn, "Cancelled during warmup, aborting start");
        capture::stop_capture(&app_inner).await;
        restore_output_volume_for(&app_inner, my_epoch).await;
        set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
        overlay::hide(&app_handle);
    }
}

/// Stop recording and finalize (paste text). Used by both toggle and hold modes.
pub(super) async fn stop_recording(app_handle: AppHandle) {
    let app_inner = app_handle.state::<Arc<app_state::AppInner>>();
    let recording_state = app_handle.state::<RecordingState>();

    // Claim this recording exactly once before any await.
    {
        let mut recording = recording_state.0.lock().unwrap();
        if !*recording {
            return;
        }
        *recording = false;
    }
    *app_inner.stop_started_at.lock().unwrap() = Some(std::time::Instant::now());
    let my_epoch = app_inner
        .session_epoch
        .load(std::sync::atomic::Ordering::SeqCst);
    let server_final_only = app_inner
        .config_manager
        .load_config()
        .is_ok_and(|config| config.audio_provider() == crate::config::ELEVENLABS_REALTIME_ID);

    // 1. Set state to finishing
    set_app_state(&app_handle, &app_inner, app_state::AppState::Finishing).await;

    // 2. Stop renderer audio first so the final buffered chunk is flushed.
    capture::stop_capture(&app_inner).await;
    if !is_current_epoch(&app_inner, my_epoch) {
        return;
    }
    // Snapshot whether real sound was captured before save_recording_wav drains
    // the buffer: a silent stop ends immediately, but speech whose transcript was
    // lost (slow/failed network) must keep the retry path even with no result yet.
    let captured_audio_signal = {
        let audio = app_inner.recording_audio.lock().await;
        wav::recording_has_audio_signal(&audio)
    };
    history::save_recording_wav(&app_handle, &app_inner).await;
    if !is_current_epoch(&app_inner, my_epoch) {
        return;
    }

    // 3. Acquire the ready ASR session. If the background connect hasn't finished
    //    (user stopped before it was ready), wait for it to resolve so the buffered
    //    audio still gets transcribed instead of being thrown away.
    let mut connection_error = None;
    let session = match app_inner.asr_session.lock().await.clone() {
        Some(s) => Some(s),
        None => {
            // If the recording was too short to contain speech and nothing was
            // recognized, cancel the in-flight connect rather than waiting up to
            // 12 s for a session that will only time out on the server side.
            // Genuine speech (signal present) still waits so the buffered audio
            // gets transcribed instead of being thrown away.
            if !server_final_only && !captured_audio_signal {
                let prefix = app_inner.accumulated_text.lock().await.clone();
                if prefix.trim().is_empty() {
                    log_rec!(
                        info,
                        "Stop with no speech signal; cancelling in-flight connect"
                    );
                    app_inner.pending_audio.lock().await.clear();
                    history::discard_recording_artifacts(&app_inner).await;
                    restore_output_volume_for(&app_inner, my_epoch).await;
                    // Cancel any in-flight connect only after the session's
                    // volume cleanup, so the scoped restore still matches.
                    app_inner
                        .session_epoch
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    overlay::hide(&app_handle);
                    set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
                    return;
                }
            }
            let rx = app_inner.connect_rx.lock().await.take();
            match rx {
                Some(rx) => match tokio::time::timeout(Duration::from_secs(12), rx).await {
                    Ok(Ok(Ok(()))) => app_inner.asr_session.lock().await.clone(),
                    Ok(Ok(Err(message))) => {
                        connection_error = Some(message);
                        None
                    }
                    Ok(Err(_)) => {
                        connection_error = Some("语音连接任务已结束，请重试".into());
                        None
                    }
                    Err(_) => {
                        connection_error = Some("语音服务连接超时，请重试".into());
                        None
                    }
                },
                None => None,
            }
        }
    };
    *app_inner.asr_events.lock().await = None;

    if let Some(session) = session {
        // 4. No speech case: the session connected but produced no transcript
        //    (no partial/final this session, nothing accumulated across reconnects)
        //    AND the captured audio was silent. The user stopped without speaking;
        //    Doubao won't emit a final for silence, so committing would block until
        //    the timeout and then wrongly offer a retry. End immediately, ESC-like.
        //    If audio WAS captured but no transcript arrived (slow/failed network),
        //    fall through to commit so the result — or a retry — is still possible.
        let recognized_anything = {
            let (final_t, partial_t) = app_inner.latest_transcript.lock().await.clone();
            let accumulated = app_inner.accumulated_text.lock().await.clone();
            !final_t.trim().is_empty()
                || !partial_t.trim().is_empty()
                || !accumulated.trim().is_empty()
        };
        if !server_final_only && !recognized_anything && !captured_audio_signal {
            log_rec!(info, "Stop with no recognized speech; ending immediately");
            session.close();
            app_inner.pending_audio.lock().await.clear();
            *app_inner.accumulated_text.lock().await = String::new();
            history::discard_recording_artifacts(&app_inner).await;
            restore_output_volume_for(&app_inner, my_epoch).await;
            overlay::hide(&app_handle);
            set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
            return;
        }

        if !is_current_epoch(&app_inner, my_epoch) {
            session.close();
            return;
        }
        // 5. Commit and get this session's final text.
        let session_text = match session.commit_and_await_final().await {
            Ok(t) => t,
            Err(e) => {
                if !is_current_epoch(&app_inner, my_epoch) {
                    session.close();
                    return;
                }
                log_rec!(warn, "ASR commit failed: {}", e);
                session.close();
                {
                    let mut slot = app_inner.asr_session.lock().await;
                    if !is_current_epoch(&app_inner, my_epoch) {
                        return;
                    }
                    slot.take();
                }
                app_inner.pending_audio.lock().await.clear();
                *app_inner.accumulated_text.lock().await = String::new();
                history::record_transcription_failure(&app_handle, &app_inner, &e).await;
                if !is_current_epoch(&app_inner, my_epoch) {
                    return;
                }
                restore_output_volume_for(&app_inner, my_epoch).await;
                overlay::emit_retryable_error_hint(&app_handle, &app_inner, &e).await;
                set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
                overlay::schedule_retry_overlay_hide(app_handle.clone(), Arc::clone(&app_inner));
                return;
            }
        };
        if !is_current_epoch(&app_inner, my_epoch) {
            session.close();
            return;
        }
        // Timing-only diagnostic: stop → final ASR result. This includes any
        // wait for a still-connecting session, so it is an end-to-end number,
        // not a pure decode benchmark.
        if let Some(started) = *app_inner.stop_started_at.lock().unwrap() {
            log_rec!(
                info,
                "ASR final result ready {} ms after stop",
                started.elapsed().as_millis()
            );
        }
        log_rec!(
            debug,
            "ASR commit final text ({} chars): {:?}",
            session_text.chars().count(),
            session_text.chars().take(200).collect::<String>()
        );

        // Prepend any text accumulated across reconnects in this recording.
        let prefix = app_inner.accumulated_text.lock().await.clone();
        let combined = format!("{}{}", prefix, session_text);

        session.close();
        {
            let mut slot = app_inner.asr_session.lock().await;
            if !is_current_epoch(&app_inner, my_epoch) {
                return;
            }
            slot.take();
        }
        // 5-9. Polish (if applicable), write clipboard, paste, record stats, end cue.
        let outcome =
            finalize::finalize_and_paste(&app_handle, &app_inner, combined, my_epoch).await;

        if !is_current_epoch(&app_inner, my_epoch) {
            return;
        }

        if outcome == finalize::FinalizeOutcome::Deferred {
            // A failed polish parked the text and already settled the app state:
            // leave the overlay up so the user can retry, ESC out, or let it
            // auto-paste.
            *app_inner.accumulated_text.lock().await = String::new();
            return;
        }
        if outcome == finalize::FinalizeOutcome::Picker {
            // The picker owns delivery now. Keep the volume lowered and the app
            // state as-is until the user selects or dismisses it.
            *app_inner.accumulated_text.lock().await = String::new();
            return;
        }
    } else {
        if !is_current_epoch(&app_inner, my_epoch) {
            return;
        }
        finalize::finalize_on_failure(
            &app_handle,
            &app_inner,
            connection_error
                .as_deref()
                .unwrap_or("语音服务未能完成最终确认，请重试"),
            my_epoch,
        )
        .await;
        return;
    }

    // 11. Clear cross-reconnect accumulated text for the next recording.
    *app_inner.accumulated_text.lock().await = String::new();

    // The workflow (ASR, polish, clipboard, paste) is complete; restore before
    // returning to Idle so a new session cannot race this cleanup.
    restore_output_volume_for(&app_inner, my_epoch).await;

    // Successful output owns its feedback lifetime.
    if !app_inner
        .feedback_visible
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        overlay::hide(&app_handle);
        set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;
    }
}

/// Tear down an in-flight retry or a shown retryable failure: discard any
/// in-flight result via the epoch bump, clear retry state, and hide the overlay.
pub(super) async fn abort_retry_or_failure(
    app_handle: &AppHandle,
    app_inner: &Arc<app_state::AppInner>,
) {
    let abort_epoch = app_inner
        .session_epoch
        .load(std::sync::atomic::Ordering::SeqCst);
    app_inner
        .session_epoch
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(session) = app_inner.asr_session.lock().await.take() {
        session.close();
    }
    capture::stop_capture(app_inner).await;
    app_inner.pending_audio.lock().await.clear();
    *app_inner.current_retry_of.lock().await = None;
    *app_inner.current_failure_ts.lock().await = None;
    *app_inner.latest_transcript.lock().await = (String::new(), String::new());
    *app_inner.accumulated_text.lock().await = String::new();
    restore_output_volume_for(app_inner, abort_epoch).await;
    let _ = app_handle.emit("overlay:event", serde_json::json!({ "type": "reset" }));
    overlay::hide(app_handle);
    // set_app_state(Idle) also re-syncs (disables) the ESC shortcut.
    set_app_state(app_handle, app_inner, app_state::AppState::Idle).await;
}

pub(super) async fn cancel_recording(app_handle: AppHandle) {
    let app_inner = app_handle.state::<Arc<app_state::AppInner>>();
    let recording_state = app_handle.state::<RecordingState>();

    let should_cancel = {
        let recording = recording_state.0.lock().unwrap();
        *recording
    };
    if !should_cancel {
        return;
    }

    *recording_state.0.lock().unwrap() = false;
    log_rec!(debug, "Cancel requested");

    // Remember which session owns the volume snapshot before bumping the epoch.
    let cancel_epoch = app_inner
        .session_epoch
        .load(std::sync::atomic::Ordering::SeqCst);

    // Bump the epoch so any in-flight background connect task discards its
    // result, and drop any audio buffered before the session was ready.
    app_inner
        .session_epoch
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    app_inner.pending_audio.lock().await.clear();
    app_inner.recording_audio.lock().await.clear();
    *app_inner.current_recording_wav.lock().await = None;
    *app_inner.current_retry_of.lock().await = None;

    // Clear the active prompt ID since the session was cancelled
    if let Some(active) = app_handle.try_state::<ActivePromptId>() {
        *active.0.lock().unwrap() = None;
    }

    capture::stop_capture(&app_inner).await;

    if let Some(session) = app_inner.asr_session.lock().await.take() {
        session.close();
    }
    *app_inner.asr_events.lock().await = None;
    *app_inner.latest_transcript.lock().await = (String::new(), String::new());
    *app_inner.accumulated_text.lock().await = String::new();

    restore_output_volume_for(&app_inner, cancel_epoch).await;
    let _ = app_handle.emit("overlay:event", serde_json::json!({ "type": "reset" }));
    overlay::hide(&app_handle);
    set_app_state(&app_handle, &app_inner, app_state::AppState::Idle).await;

    // Resync the hotkey matcher: the recording was cancelled out-of-band
    // (ESC), so it must not still believe a session is active.
    reset_matcher_recording(&app_handle);
}
