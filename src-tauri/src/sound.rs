//! Audio file playback via rodio (macOS + Windows).
//!
//! The single in-process playback path for all sounds: recording cues and the
//! settings "preview" button. Replaces the former per-platform process spawns
//! (afplay / PowerShell `SoundPlayer`) that lived in `paste`. Playing in the
//! long-lived backend process means a sound is never cut short by the overlay
//! WebView being torn down, and there is no decode/keep-alive jank. rodio
//! decodes via symphonia (mp3 out of the box) and handles resampling internally,
//! so no sample-rate matching is needed.

use std::time::Duration;

/// Completion signal for a cue started by [`play_audio_file`].
///
/// Recording-start dimming must not lower the system volume while the cue is
/// still audible. rodio exposes playback completion through
/// `Sink::sleep_until_end`, so the playback thread resolves this once the sound
/// has finished, failed, or been superseded. Waiting is always bounded by the
/// caller so a pathological file can never stall the recording workflow.
pub(crate) struct PlaybackWaiter {
    done: tokio::sync::oneshot::Receiver<()>,
}

impl PlaybackWaiter {
    /// Wait for playback to end, at most `timeout`. A timeout means the cue is
    /// still playing (or the thread is stuck); the caller proceeds anyway.
    pub(crate) async fn wait(self, timeout: Duration) {
        let _ = tokio::time::timeout(timeout, self.done).await;
    }
}

/// The cue currently playing, if any. Cues are single-instance: starting a new
/// one stops the previous sound so two shortcuts in quick succession can never
/// overlap. Only the rodio `Sink` is stored (it is `Send`); the `OutputStream`
/// has to stay on its playback thread.
fn cue_slot() -> &'static std::sync::Mutex<Option<std::sync::Arc<rodio::Sink>>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::Arc<rodio::Sink>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

/// Generation counter for cue requests: a superseded cue must not re-register
/// itself in the slot after a newer cue has already started.
fn cue_generation() -> &'static std::sync::atomic::AtomicU64 {
    static GENERATION: std::sync::OnceLock<std::sync::atomic::AtomicU64> =
        std::sync::OnceLock::new();
    GENERATION.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn stop_cue() {
    let playing = cue_slot().lock().ok().and_then(|mut slot| slot.take());
    if let Some(sink) = playing {
        sink.stop();
    }
}

/// Play a cue on a background thread. The thread blocks on `sleep_until_end` so
/// the `OutputStream` stays alive for the whole sound, then resolves the
/// returned waiter. Errors are logged only — rodio + symphonia is stable enough
/// that no fallback (afplay/PowerShell) is warranted. Empty path is a no-op.
pub(crate) fn play_audio_file(file_path: &str) -> Option<PlaybackWaiter> {
    if file_path.is_empty() {
        return None;
    }

    let generation = cue_generation().fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    stop_cue();

    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let path = file_path.to_string();
    if let Err(error) = std::thread::Builder::new()
        .name("voicepaste-sound".to_string())
        .spawn(move || {
            if let Err(error) = play_cue(&path, generation) {
                log_app!(warn, "Audio playback failed ({}): {}", path, error);
            }
            // Resolve on every path (played, failed, or superseded) so the
            // caller's bounded wait never has to guess.
            let _ = done_tx.send(());
        })
    {
        log_app!(warn, "Failed to spawn audio thread: {}", error);
        return None;
    }

    Some(PlaybackWaiter { done: done_rx })
}

fn play_cue(path: &str, my_generation: u64) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::BufReader;
    use std::sync::atomic::Ordering::SeqCst;

    // Keep `_stream` alive for the whole call: dropping it stops playback.
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = std::sync::Arc::new(rodio::Sink::try_new(&handle)?);
    // Decode setup happens outside the slot lock so a bad file cannot hold it.
    let decoder = rodio::Decoder::new(BufReader::new(File::open(path)?))?;

    {
        let mut slot = cue_slot()
            .lock()
            .map_err(|_| "cue slot poisoned".to_string())?;
        if cue_generation().load(SeqCst) != my_generation {
            // A newer cue superseded this one while it was starting up.
            return Ok(());
        }
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = Some(std::sync::Arc::clone(&sink));
        // Append inside the lock so `stop_cue` cannot see the sink before the
        // audio is queued.
        sink.append(decoder);
    }
    sink.sleep_until_end();

    // Release the slot only if it still holds this sink.
    if let Ok(mut slot) = cue_slot().lock() {
        let is_current = slot
            .as_ref()
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, &sink));
        if is_current {
            *slot = None;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings preview playback (stoppable, single-instance)
// ---------------------------------------------------------------------------

/// The settings preview currently playing, if any.
///
/// Recording cues stay fire-and-forget, but the history list's play button needs
/// to be stoppable and must never stack: clicking two entries in a row used to
/// start a second detached thread, so both recordings played at once with no way
/// to stop either. Only the rodio `Sink` is kept here — it is a `Send` handle,
/// whereas the `OutputStream` is not and has to stay on the thread that owns it.
fn preview_slot() -> &'static std::sync::Mutex<Option<std::sync::Arc<rodio::Sink>>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::Arc<rodio::Sink>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

/// Stop the preview currently playing, if any.
pub(crate) fn stop_preview() {
    let playing = preview_slot().lock().ok().and_then(|mut slot| slot.take());
    if let Some(sink) = playing {
        sink.stop();
    }
}

/// How a preview playback ended. `Superseded` covers both "stopped by the
/// user" and "replaced by a newer preview" — in both cases the *caller* owns
/// the UI state, so no end callback fires.
enum PreviewEnd {
    /// Played to natural completion.
    Played,
    /// Stopped or replaced before finishing.
    Superseded,
}

/// Generation counter for preview requests. Each `play_preview_file` bumps it;
/// a slower earlier thread checks it inside the slot lock and stands down so a
/// double click can never end up with two sinks audible.
fn preview_generation() -> &'static std::sync::atomic::AtomicU64 {
    static GENERATION: std::sync::OnceLock<std::sync::atomic::AtomicU64> =
        std::sync::OnceLock::new();
    GENERATION.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

/// Play `file_path` as the settings preview, replacing whatever was playing.
/// `on_end` runs when playback finishes on its own or fails to start, so the UI
/// can return its button to the play state. It does NOT run when this preview
/// is stopped or replaced — the action that stopped it already reset the UI.
/// Empty path is a no-op (and still fires `on_end`).
pub(crate) fn play_preview_file<F>(file_path: &str, on_end: F)
where
    F: FnOnce() + Send + 'static,
{
    let generation = preview_generation().fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    stop_preview();
    if file_path.is_empty() {
        on_end();
        return;
    }

    // The callback has to be reachable from both the playback thread and the
    // spawn-failure path here, and it can only run once — hence the shared slot
    // rather than passing it straight into the closure. Whichever side gets
    // there first takes it; the UI is never left stuck on "playing".
    type EndCallback = std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>;
    fn fire(slot: &EndCallback) {
        let callback = slot.lock().ok().and_then(|mut slot| slot.take());
        if let Some(callback) = callback {
            callback();
        }
    }

    let on_end: std::sync::Arc<EndCallback> =
        std::sync::Arc::new(std::sync::Mutex::new(Some(Box::new(on_end))));
    let thread_on_end = std::sync::Arc::clone(&on_end);
    let path = file_path.to_string();
    if let Err(error) = std::thread::Builder::new()
        .name("voicepaste-preview".to_string())
        .spawn(move || match play_preview(&path, generation) {
            Ok(PreviewEnd::Played) => fire(&thread_on_end),
            Ok(PreviewEnd::Superseded) => {}
            Err(error) => {
                // A failed playback still has to reset the UI button.
                log_app!(warn, "Preview playback failed ({}): {}", path, error);
                fire(&thread_on_end);
            }
        })
    {
        log_app!(warn, "Failed to spawn preview thread: {}", error);
        fire(&on_end);
    }
}

fn play_preview(path: &str, my_generation: u64) -> Result<PreviewEnd, Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::BufReader;
    use std::sync::atomic::Ordering::SeqCst;
    // Keep `_stream` alive for the whole call: dropping it stops playback.
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = std::sync::Arc::new(rodio::Sink::try_new(&handle)?);
    // Decode setup happens outside the slot lock so a bad file cannot hold it.
    let decoder = rodio::Decoder::new(BufReader::new(File::open(path)?))?;

    {
        let mut slot = preview_slot().lock().map_err(|_| "preview slot poisoned")?;
        // A newer request superseded this one while it was starting up.
        if preview_generation().load(SeqCst) != my_generation {
            return Ok(PreviewEnd::Superseded);
        }
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = Some(std::sync::Arc::clone(&sink));
        // Append inside the lock: appending after registration would leave a
        // window where stop_preview sees the sink but the sound has not been
        // queued yet, so stopping it would miss the audio entirely.
        sink.append(decoder);
    }
    sink.sleep_until_end();

    // Take the slot back only if it still holds *this* sink: a newer preview or
    // an explicit stop replaced it while this one was winding down.
    let still_mine = preview_slot()
        .lock()
        .ok()
        .map(|mut slot| {
            let is_current = slot
                .as_ref()
                .is_some_and(|current| std::sync::Arc::ptr_eq(current, &sink));
            if is_current {
                *slot = None;
            }
            is_current
        })
        .unwrap_or(false);
    Ok(if still_mine {
        PreviewEnd::Played
    } else {
        PreviewEnd::Superseded
    })
}
