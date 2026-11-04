use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::asr::{AsrEvent, AsrSession};
use crate::config::ConfigManager;
use crate::hotkey;
use crate::hotword::HotwordManager;
use crate::stats::StatsService;

/// Application recording state.
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Idle,
    Connecting,
    Recording,
    Finishing,
}

/// Global application state shared across all Tauri commands.
/// A polish attempt that failed and is waiting on the user. Holds everything
/// needed to either retry the polish or fall back to pasting the raw text.
#[derive(Debug, Clone)]
pub struct PendingPolish {
    /// Recognized text after ASR cleanup, ready to paste as-is on fallback.
    pub raw_text: String,
    /// Prompt template the failed polish used, so a retry uses the same one.
    pub prompt_id: String,
    /// Recording epoch that owns the lowered system volume. Kept with the
    /// parked text so a late resolution cannot restore a newer session.
    pub volume_session: u64,
}

/// Interruption for an in-flight polish retry, triggered by ESC/Enter while
/// the "润色中…" pass is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolishRetryCommand {
    /// ESC: abort the retry and discard the text (no paste, no history entry).
    Cancel,
    /// Enter: abort the retry and paste the raw text immediately.
    DeliverRaw,
}

pub struct AppInner {
    pub overlay_epoch: std::sync::atomic::AtomicU64,
    pub feedback_visible: std::sync::atomic::AtomicBool,
    pub recording_hints: Mutex<Option<crate::overlay::shared::RecordingHints>>,
    pub recording_mode: Mutex<hotkey::HotkeyMode>,
    pub state: Mutex<AppState>,
    pub config_manager: ConfigManager,
    pub hotword_manager: HotwordManager,
    pub log_path: std::path::PathBuf,
    pub stats: Mutex<StatsService>,
    pub asr_session: Mutex<Option<Arc<dyn AsrSession>>>,
    pub asr_events: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<AsrEvent>>>,
    pub latest_transcript: Mutex<(String, String)>, // (final_text, partial_text)
    /// Audio sample chunks captured before the ASR session is ready (background
    /// connect in progress, or during a reconnect gap). Drained into the session
    /// once it attaches. Always accessed while holding `asr_session` to stay
    /// ordered against the drain.
    pub pending_audio: Mutex<Vec<Vec<f32>>>,
    /// Full-session 16k mono PCM captured from the same stream sent to ASR.
    /// Saved as a WAV when a recording is finalized, for diagnostics and review.
    pub recording_audio: Mutex<Vec<f32>>,
    /// WAV path for the current recording once saved.
    pub current_recording_wav: Mutex<Option<std::path::PathBuf>>,
    /// History timestamp of the failed entry currently being retried.
    pub current_retry_of: Mutex<Option<String>>,
    /// Latest failed history entry that has a WAV and can be retried from the overlay.
    pub current_failure_ts: Mutex<Option<String>>,
    /// Resolves when the background ASR connect finishes (Ok) or fails (Err).
    /// `stop_recording` awaits this when the user stops before the session is ready.
    pub connect_rx: Mutex<Option<tokio::sync::oneshot::Receiver<Result<(), String>>>>,
    /// Recording-session generation. Bumped on each start and on cancel so a
    /// stale background connect task (from a cancelled/superseded session) can
    /// detect it is obsolete and discard its result.
    pub session_epoch: std::sync::atomic::AtomicU64,
    /// Finalized text carried across ASR reconnects within a single recording.
    /// Each reconnect starts a fresh server-side session with no memory of prior
    /// audio, so already-recognized text is accumulated here and prepended to the
    /// new session's output. Reset at the start of every recording.
    pub accumulated_text: Mutex<String>,
    /// Native microphone capture (cpal — CoreAudio on macOS, WASAPI on Windows,
    /// ALSA on Linux). The overlay renderer no longer captures audio.
    pub native_audio: Mutex<Option<crate::recording::NativeAudioCapture>>,
    /// A failed LLM polish awaiting the user's decision. While this is set the
    /// recognized text has NOT been pasted yet: the overlay shows why polishing
    /// failed, the main hotkey retries, ESC discards the text,
    /// Enter pastes the raw text immediately, and a timer delivers it
    /// automatically if the user does nothing.
    pub pending_polish: Mutex<Option<PendingPolish>>,
    /// Serializes the deferred-polish resolution paths (auto-deliver timer,
    /// Enter/ESC, retry, and a new session settling an old parked text) so only
    /// one of them can own the parked text and run its volume restore plus
    /// overlay dismissal to completion.
    pub polish_delivery_lock: Mutex<()>,
    /// Generation counter for `pending_polish`. The auto-deliver timer captures
    /// the epoch it was armed with and stands down if a retry or ESC bumped it.
    pub polish_epoch: std::sync::atomic::AtomicU64,
    /// Whether a polish retry is currently in flight (between taking the
    /// pending text and delivering/re-arming it). Hotkey routing consults this
    /// to ignore presses and stray stops while the retry runs.
    pub polish_retry_active: std::sync::atomic::AtomicBool,
    /// Sender half of the interruption channel owned by the in-flight polish
    /// retry task. The receiver lives only inside that task, so commands sent
    /// after it finished are dropped with it.
    pub polish_retry_cmd_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<PolishRetryCommand>>>,
    /// Shared sound detector, reset between recording sessions.
    pub wave_activity: Mutex<crate::overlay::shared::WaveActivity>,
    /// Lowers the macOS output volume for the duration of a recording session
    /// and restores the exact state captured at the start. Shared across the
    /// lifecycle, deferred-polish decision window, and app exit cleanup.
    pub volume_guard: std::sync::Arc<crate::system_volume::VolumeGuard>,
    /// Display/application captured when the current session started. The
    /// indicator and the version picker both use it, and it is re-captured on
    /// every start so moving the target app between screens is followed.
    pub session_target: Mutex<Option<crate::display::TargetDisplay>>,
    /// Multi-version picker state for the active or most recent session. Results
    /// from older sessions are rejected inside the store.
    pub picker: crate::picker::model::PickerStore,
    /// Recognizers kept alive across recordings while voice input is active.
    /// Unloaded on app exit, whenever the ASR configuration changes, and after
    /// a long idle stretch (see [`schedule_idle_model_unload`]).
    pub asr_models: Arc<crate::asr::SherpaModelCache>,
    /// Generation guard for the pending idle model-unload task. Bumped on
    /// every state transition so a scheduled unload only fires if the app
    /// stayed Idle (no new recording) for the whole window.
    pub model_idle_ticket: std::sync::atomic::AtomicU64,
    /// When the current stop was requested. Used only for latency diagnostics;
    /// never contains audio or transcript content.
    pub stop_started_at: std::sync::Mutex<Option<std::time::Instant>>,
    /// When microphone capture actually opened for this session. The dashboard's
    /// "recording time" excludes ASR/LLM/picker latency and the startup cue.
    pub capture_started_at: std::sync::Mutex<Option<std::time::Instant>>,
    /// Microphone time measured when capture closed. Read by the delivery path
    /// (including a picker selection that happens much later).
    pub session_recording_ms: std::sync::Mutex<u64>,
}

pub type AppHandle = Arc<AppInner>;

/// Create the shared application state.
pub fn create_app_state(
    config_manager: ConfigManager,
    hotword_manager: HotwordManager,
    log_path: std::path::PathBuf,
    stats_service: StatsService,
) -> AppHandle {
    Arc::new(AppInner {
        overlay_epoch: std::sync::atomic::AtomicU64::new(0),
        feedback_visible: std::sync::atomic::AtomicBool::new(false),
        recording_hints: Mutex::new(None),
        recording_mode: Mutex::new(hotkey::HotkeyMode::Toggle),
        state: Mutex::new(AppState::Idle),
        config_manager,
        hotword_manager,
        log_path,
        stats: Mutex::new(stats_service),
        asr_session: Mutex::new(None),
        asr_events: Mutex::new(None),
        latest_transcript: Mutex::new((String::new(), String::new())),
        pending_audio: Mutex::new(Vec::new()),
        recording_audio: Mutex::new(Vec::new()),
        current_recording_wav: Mutex::new(None),
        current_retry_of: Mutex::new(None),
        current_failure_ts: Mutex::new(None),
        connect_rx: Mutex::new(None),
        session_epoch: std::sync::atomic::AtomicU64::new(0),
        accumulated_text: Mutex::new(String::new()),
        native_audio: Mutex::new(None),
        pending_polish: Mutex::new(None),
        polish_delivery_lock: Mutex::new(()),
        polish_epoch: std::sync::atomic::AtomicU64::new(0),
        polish_retry_active: std::sync::atomic::AtomicBool::new(false),
        polish_retry_cmd_tx: Mutex::new(None),
        wave_activity: Mutex::new(crate::overlay::shared::WaveActivity::default()),
        volume_guard: crate::system_volume::VolumeGuard::new_default(),
        session_target: Mutex::new(None),
        picker: crate::picker::model::PickerStore::default(),
        asr_models: Arc::new(crate::asr::SherpaModelCache::new()),
        model_idle_ticket: std::sync::atomic::AtomicU64::new(0),
        stop_started_at: std::sync::Mutex::new(None),
        capture_started_at: std::sync::Mutex::new(None),
        session_recording_ms: std::sync::Mutex::new(0),
    })
}

// ---------------------------------------------------------------------------
// Recording state machine helpers
// ---------------------------------------------------------------------------

/// How long the app must stay Idle before the cached ASR recognizers are
/// released. The parakeet-class model set is ~700 MB resident; an always-on
/// tray app otherwise carries it through every idle hour. Reloading costs a
/// few seconds, and audio captured while a cold session connects is buffered
/// (up to ~30 s in `pending_audio`), so nothing the user says is lost.
const ASR_MODEL_IDLE_UNLOAD_SECS: u64 = 300;

/// Profiling/testing override for the idle unload window (seconds). Lower it
/// to observe the release in a memory profile without waiting five minutes;
/// unset keeps the production window.
fn asr_model_idle_unload_secs() -> u64 {
    std::env::var("VOICEPASTE_ASR_IDLE_UNLOAD_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(ASR_MODEL_IDLE_UNLOAD_SECS)
}

/// (Re)arm the idle model unload. Every state transition bumps the ticket, so
/// an in-flight sleep either finds its ticket stale (a newer transition
/// happened) or re-checks the live state before releasing anything. An active
/// session holds its own `Arc` to the recognizer, so unloading the cache
/// never yanks a model out from under a recording.
pub(crate) fn schedule_idle_model_unload(app_inner: &Arc<AppInner>) {
    use std::sync::atomic::Ordering;
    let ticket = app_inner.model_idle_ticket.fetch_add(1, Ordering::SeqCst) + 1;
    let inner = Arc::clone(app_inner);
    tauri::async_runtime::spawn(async move {
        let secs = asr_model_idle_unload_secs();
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        if inner.model_idle_ticket.load(Ordering::SeqCst) != ticket {
            // A newer state transition superseded this window.
            return;
        }
        if *inner.state.lock().await != AppState::Idle {
            return;
        }
        if inner.asr_session.lock().await.is_some() {
            return;
        }
        inner.asr_models.unload(&format!("idle for {secs}s"));
    });
}

/// Map an [`AppState`] to the string label emitted to the overlay and logged.
pub(crate) fn app_state_name(state: &AppState) -> &'static str {
    match state {
        AppState::Idle => "idle",
        AppState::Connecting => "connecting",
        AppState::Recording => "recording",
        AppState::Finishing => "finishing",
    }
}

/// ESC-cancel is only meaningful while a recording is in flight (or being
/// finalized). Idle keeps ESC off so it doesn't swallow ordinary key events.
pub(crate) fn should_enable_escape_shortcut(state: &AppState) -> bool {
    matches!(
        state,
        AppState::Connecting | AppState::Recording | AppState::Finishing
    )
}

/// Enable/disable the global ESC shortcut to match the new recording state.
pub(crate) fn sync_escape_shortcut(app: &tauri::AppHandle, state: &AppState) {
    if let Some(hc) = app.try_state::<hotkey::HotkeyConfig>() {
        hotkey::set_escape_enabled(&hc, should_enable_escape_shortcut(state));
    }
}

/// Transition to `next_state`: persist it, reset waveform smoothing when leaving
/// Recording, sync the ESC shortcut, and broadcast the change to the overlay.
pub(crate) async fn set_app_state(
    app: &tauri::AppHandle,
    app_inner: &Arc<AppInner>,
    next_state: AppState,
) {
    if next_state != AppState::Idle {
        crate::overlay::invalidate_feedback(app_inner);
        crate::overlay::emit_shortcuts(app, &[]);
    }
    if next_state != AppState::Recording {
        *app_inner.recording_hints.lock().await = None;
    }
    *app_inner.state.lock().await = next_state.clone();
    // Reset sound activity so a new session starts with resting bars.
    if next_state != AppState::Recording {
        *app_inner.wave_activity.lock().await = crate::overlay::shared::WaveActivity::default();
    }
    sync_escape_shortcut(app, &next_state);

    // Model lifecycle follows the session lifecycle: starting a new state
    // invalidates any pending idle unload; reaching Idle arms a fresh one.
    if next_state == AppState::Idle {
        schedule_idle_model_unload(app_inner);
    } else {
        app_inner
            .model_idle_ticket
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    let _ = app.emit(
        "overlay:event",
        serde_json::json!({
            "type": "state",
            "payload": { "state": app_state_name(&next_state) }
        }),
    );
    log_rec!(info, "State → {}", app_state_name(&next_state));
}

// ---------------------------------------------------------------------------
// Tauri managed-state wrappers
// ---------------------------------------------------------------------------

/// Wrapper to keep the HotkeyManager alive as Tauri managed state.
/// The `_inner` field is intentionally never read — its purpose is to hold
/// ownership of the HotkeyManager so its Drop stops the keytap listener.
pub struct HotkeyManagerState {
    pub _inner: std::sync::Mutex<hotkey::HotkeyManager>,
}

/// Simple recording toggle state managed by Tauri.
pub struct RecordingState(pub std::sync::Mutex<bool>);

/// Hotkey mode: "toggle" (press once to start, press again to stop) or "hold" (hold to speak).
pub struct HotkeyMode(pub std::sync::Mutex<String>);

/// Tracks which prompt template triggered the current recording session.
/// `None` means the main hotkey was used (not a prompt-specific hotkey).
pub struct ActivePromptId(pub std::sync::Mutex<Option<String>>);
