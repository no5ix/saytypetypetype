//! Finalize-and-paste finishing flow: optional LLM polishing, trailing-period
//! trimming, clipboard write, simulated paste, usage stats, and the end cue.
//! Also handles the salvage path when a recording fails mid-stream.

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::app_state::{self, set_app_state, ActivePromptId, RecordingState};
use crate::overlay;

use super::capture;
use super::cue;
use super::history;
use super::polish;
use super::{
    await_output_volume_settled, emit_hint, reset_matcher_recording, restore_output_volume_for,
};

/// Whether the finishing pipeline actually delivered text, or parked it awaiting
/// the user's decision after a failed polish. `Deferred` keeps the overlay up:
/// the caller must not hide it or clear the failure affordance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalizeOutcome {
    Delivered,
    /// A failed polish parked the text awaiting the user's decision.
    Deferred,
    /// The multi-version picker is showing; delivery happens on selection.
    Picker,
    OutputFailed,
    Cancelled,
}

/// A failed recognition never enters the paste pipeline. Retain the WAV for an
/// explicit retry; temporary and incomplete text must not masquerade as final.
pub(super) async fn finalize_on_failure(
    app: &AppHandle,
    app_inner: &Arc<app_state::AppInner>,
    message: &str,
    expected_epoch: u64,
) {
    if !super::is_current_epoch(app_inner, expected_epoch) {
        return;
    }
    if let Some(state) = app.try_state::<RecordingState>() {
        *state.0.lock().unwrap() = false;
    }
    reset_matcher_recording(app);
    if let Some(session) = app_inner.asr_session.lock().await.take() {
        session.close();
    }
    capture::stop_capture(app_inner).await;
    if !super::is_current_epoch(app_inner, expected_epoch) {
        return;
    }
    history::save_recording_wav(app, app_inner).await;
    if !super::is_current_epoch(app_inner, expected_epoch) {
        return;
    }
    *app_inner.accumulated_text.lock().await = String::new();
    *app_inner.latest_transcript.lock().await = (String::new(), String::new());
    *app_inner.asr_events.lock().await = None;
    app_inner.pending_audio.lock().await.clear();
    history::record_transcription_failure(app, app_inner, message).await;
    if !super::is_current_epoch(app_inner, expected_epoch) {
        return;
    }
    if let Some(active) = app.try_state::<ActivePromptId>() {
        *active.0.lock().unwrap() = None;
    }
    // Terminal failure: restore the volume before returning to Idle. This is
    // also the path used by the background ASR session manager when a fatal
    // error ends an active recording.
    restore_output_volume_for(app_inner, expected_epoch).await;
    overlay::emit_retryable_error_hint(app, app_inner, message).await;
    set_app_state(app, app_inner, app_state::AppState::Idle).await;
    overlay::schedule_retry_overlay_hide(app.clone(), Arc::clone(app_inner));
}

/// Run recognized text through the finishing pipeline: optional LLM polishing
/// (prompt-specific hotkeys only, with sherpa-onnx hotword hinting), trailing-
/// period trimming, clipboard write (honoring keep_clipboard), simulated paste,
/// usage stats, and the end cue. Only confirmed results from normal stops and
/// explicit retries may reach this pipeline.
pub(super) async fn finalize_and_paste(
    app_handle: &AppHandle,
    app_inner: &Arc<app_state::AppInner>,
    raw_text: String,
    expected_epoch: u64,
) -> FinalizeOutcome {
    if !super::is_current_epoch(app_inner, expected_epoch) {
        return FinalizeOutcome::Cancelled;
    }
    let trimmed = raw_text.trim().to_string();

    // Always clear the active prompt ID once a recording concludes.
    let active_prompt_id = app_handle
        .try_state::<ActivePromptId>()
        .and_then(|s| s.0.lock().unwrap().clone());
    if let Some(active) = app_handle.try_state::<ActivePromptId>() {
        *active.0.lock().unwrap() = None;
    }

    if trimmed.is_empty() {
        log_rec!(warn, "Final text is empty, skipping paste");
        return FinalizeOutcome::Delivered;
    }

    log_rec!(
        info,
        "Final text received ({} chars)",
        trimmed.chars().count()
    );
    log_rec!(
        debug,
        "Final text preview: {:?}",
        trimmed.chars().take(200).collect::<String>()
    );

    // Load config + model registry for LLM / behavior settings.
    let config = app_inner.config_manager.load_config().ok();

    let resource_dir = app_handle
        .path()
        .resource_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let registry = crate::model::load_registry(&data_dir, &resource_dir);

    let mut trimmed = trimmed;
    // Restore hotword casing for engines configured to do so (e.g. sherpa-onnx
    // lowercases proper nouns recognized via its hotword list).
    if let Some(ref config) = config {
        let model_id = config.audio_provider();
        if config.hotword_replace(model_id, &registry) {
            let hotwords = app_inner.hotword_manager.active_words();
            if !hotwords.is_empty() {
                trimmed = crate::asr::apply_post_asr_corrections(&trimmed, &hotwords);
            }
        }
    }

    if config
        .as_ref()
        .map(|config| config.app.remove_trailing_period)
        .unwrap_or(true)
        && (trimmed.ends_with('。') || trimmed.ends_with('.'))
    {
        trimmed.pop();
    }

    // Multi-version picker: when enabled, the original transcript plus every
    // configured template is offered as a choice instead of auto-pasting the
    // one template that the stop chord happened to select. Falls back to the
    // existing single-result flow when there are no templates to compare.
    let picker_prompts = app_inner.config_manager.load_prompts();
    let picker_enabled = cfg!(target_os = "macos")
        && config
            .as_ref()
            .map(|config| config.app.multi_version_picker)
            .unwrap_or(false);
    if picker_enabled && !picker_prompts.is_empty() {
        if let Some(config) = config.as_ref() {
            let hotwords = app_inner.hotword_manager.active_words();
            let opened = crate::picker::open_session(
                app_handle,
                app_inner,
                trimmed.clone(),
                picker_prompts,
                config,
                &registry,
                hotwords,
                expected_epoch,
            )
            .await;
            if opened {
                return FinalizeOutcome::Picker;
            }
        }
    }

    // When a prompt-specific stop chord was used, validate the LLM config before
    // calling out. A missing key/URL/model parks the text in the same decision
    // window as a failed polish (after normalization, so what gets parked is
    // exactly what would have been polished), instead of downgrading to a
    // one-frame warning followed by an unpolished paste.
    if let (Some(ref cfg), Some(prompt_id)) = (&config, &active_prompt_id) {
        if let Err(e) = crate::llm::validate_llm_config(&cfg.llm) {
            log_rec!(
                warn,
                "LLM config invalid, holding text for polish retry: {}",
                e
            );
            polish::arm_pending_polish(
                app_handle,
                app_inner,
                trimmed.clone(),
                prompt_id,
                &crate::llm::PolishError::config(e),
            )
            .await;
            return FinalizeOutcome::Deferred;
        }
    }

    // Apply LLM structure_text only when a prompt-specific hotkey was used.
    // The main hotkey (active_prompt_id = None) pastes raw text without polishing.
    let final_text = match (&config, &active_prompt_id) {
        (Some(config), Some(prompt_id)) => {
            let prompts = app_inner.config_manager.load_prompts();
            let hotwords = app_inner.hotword_manager.active_words();
            // Show a "润色中…" hint (with shimmer) while the LLM is working.
            emit_hint(app_handle, "润色中…", "info", "progress");
            let outcome = crate::llm::polish_transcript(
                config,
                &prompts,
                Some(prompt_id.as_str()),
                &hotwords,
                &registry,
                &trimmed,
            )
            .await;
            if !super::is_current_epoch(app_inner, expected_epoch) {
                return FinalizeOutcome::Cancelled;
            }
            match outcome {
                crate::llm::PolishOutcome::Polished(t) => {
                    log_rec!(
                        info,
                        "LLM polishing succeeded ({} chars)",
                        t.chars().count()
                    );
                    log_rec!(
                        debug,
                        "LLM polished preview: {:?}",
                        t.chars().take(200).collect::<String>()
                    );
                    t
                }
                // Do not paste yet: show why it failed and hand the user the
                // retry / ESC / auto-deliver window instead of silently
                // dropping them back to the unpolished text.
                crate::llm::PolishOutcome::Failed(e) => {
                    polish::arm_pending_polish(app_handle, app_inner, trimmed, prompt_id, &e).await;
                    return FinalizeOutcome::Deferred;
                }
                crate::llm::PolishOutcome::NotPolished => trimmed.clone(),
            }
        }
        _ => trimmed.clone(),
    };

    // Remember which template produced this session's text so the history list
    // can offer a re-polish. The history entry itself always stores the original
    // transcript (see `deliver_text`), so no polished copy is persisted here.
    let polish = crate::stats::PolishRecord {
        prompt_id: active_prompt_id.clone(),
        raw_text: None,
    };
    if !super::is_current_epoch(app_inner, expected_epoch) {
        return FinalizeOutcome::Cancelled;
    }
    if deliver_text(
        app_handle,
        app_inner,
        &final_text,
        &trimmed,
        config.as_ref(),
        polish,
        expected_epoch,
    )
    .await
    {
        FinalizeOutcome::Delivered
    } else {
        FinalizeOutcome::OutputFailed
    }
}

/// Deliver already-polished/selected text while recording the original ASR
/// transcript in history. Reports failures as a `Result` so the picker can keep
/// its own error surface instead of the overlay's error state.
pub(crate) async fn deliver_text_result(
    app_handle: &AppHandle,
    app_inner: &Arc<app_state::AppInner>,
    final_text: &str,
    history_text: &str,
    config: Option<&crate::config::AppConfig>,
    polish: crate::stats::PolishRecord,
    volume_session: u64,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    overlay::set_decision_keys_enabled(app_handle, false);

    // Both modes auto-paste the recognized text. keep_clipboard (ON) leaves the
    // recognized text in the clipboard after pasting; !keep_clipboard (OFF,
    // default) restores whatever the clipboard held beforehand.
    let keep_clipboard = config.map(|c| c.app.keep_clipboard).unwrap_or(true);

    // Snapshot the clipboard before overwriting it so we can restore it after
    // the paste. Only needed when OFF (ON keeps the recognized text).
    let original_clipboard: Option<String> = if !keep_clipboard {
        app_handle.clipboard().read_text().ok()
    } else {
        None
    };

    if let Err(e) = app_handle.clipboard().write_text(final_text) {
        return Err(format!("剪贴板写入失败: {}", e));
    }

    let result = crate::paste::simulate_paste();

    // Restore original clipboard content when keep_clipboard is OFF. When ON,
    // original_clipboard is None, so the recognized text stays in the clipboard.
    if let Some(original) = original_clipboard {
        if let Err(e) = app_handle.clipboard().write_text(&original) {
            log_rec!(error, "Failed to restore clipboard: {}", e);
        }
    }

    if !result.ok {
        return Err(result.message.unwrap_or_else(|| "模拟粘贴失败".to_string()));
    }

    // Successful delivery ends the volume session. Restore BEFORE the completion
    // cue so the end sound plays at the user's original volume instead of the
    // dimmed recording volume. Pick/session paths call restore again; the guard
    // is idempotent and session-scoped.
    restore_output_volume_for(app_inner, volume_session).await;

    // Timing-only diagnostic. For the immediate path this is stop → pasted
    // text; for picker/polish paths it also includes the user/LLM decision
    // time, so read it together with the "ASR final result ready" log.
    if let Some(started) = *app_inner.stop_started_at.lock().unwrap() {
        log_rec!(
            info,
            "Text delivered {} ms after stop",
            started.elapsed().as_millis()
        );
    }

    // Record usage stats and retain/delete the WAV according to user settings.
    // The entry stores the original transcript, never the selected/polished text.
    let keep_recordings = config.map(|c| c.app.keep_recordings).unwrap_or(false);
    history::record_success_and_apply_retention(
        app_handle,
        app_inner,
        history_text,
        keep_recordings,
        polish,
    )
    .await;
    // No post-paste success pill. It was drawn always-on-top near the bottom
    // center of the screen — right where the pasted text and the caret land —
    // so it covered the user's text. The workflow is finished: settle to Idle
    // and dismiss the recording overlay immediately instead. Failures still
    // surface through `show_output_error`, and the end cue is unchanged.
    set_app_state(app_handle, app_inner, app_state::AppState::Idle).await;
    overlay::hide(app_handle);
    // The restore runs as a ~1 s glide so it never blocks the paste or the
    // state reset. The completion cue still honours the old ordering (volume
    // restored before the cue): wait for the glide, bounded, right here.
    await_output_volume_settled(app_inner).await;
    cue::emit_cue(app_handle, app_inner, "end");
    Ok(())
}

/// Write text to the clipboard, simulate the paste keystroke, restore the
/// previous clipboard when configured, record usage stats and play the end cue.
/// Shared by the normal finish and by the deferred polish-retry paths. On
/// failure, shows the overlay's error state and returns `false`.
pub(super) async fn deliver_text(
    app_handle: &AppHandle,
    app_inner: &Arc<app_state::AppInner>,
    final_text: &str,
    history_text: &str,
    config: Option<&crate::config::AppConfig>,
    polish: crate::stats::PolishRecord,
    volume_session: u64,
) -> bool {
    match deliver_text_result(
        app_handle,
        app_inner,
        final_text,
        history_text,
        config,
        polish,
        volume_session,
    )
    .await
    {
        Ok(()) => true,
        Err(message) => {
            overlay::show_output_error(app_handle, app_inner, &message).await;
            false
        }
    }
}
