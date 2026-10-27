//! Temporary macOS system output-volume control for a transcription session.
//!
//! On session start we capture the current output volume and mute state, then
//! lower the volume to [`DIM_VOLUME`] percent when it is higher. The captured
//! state is restored exactly once when the workflow reaches any terminal
//! outcome, so a user adjustment made while transcribing is intentionally
//! overwritten by the value saved at the start.
//!
//! The lifecycle owns the begin/restore boundaries; this module only owns the
//! per-session snapshot, duplicate-cleanup protection, and the macOS I/O.
//! Volume control is best-effort: a failure is logged and never blocks a
//! transcription.

use std::sync::{Arc, Mutex};

/// Target output volume, in percent, while a transcription session is active.
pub(crate) const DIM_VOLUME: u8 = 10;

/// A snapshot of the system output state captured when a session starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputVolume {
    pub volume: u8,
    pub muted: bool,
}

impl OutputVolume {
    fn describe(&self) -> String {
        format!("{}% (muted={})", self.volume, self.muted)
    }
}

/// Platform-specific output-volume operations.
///
/// `read` returns `Ok(None)` when the platform has no system output-volume
/// control (Windows/Linux keep the feature a no-op). Tests inject a fake.
pub(crate) trait VolumeController: Send + Sync {
    fn read(&self) -> Result<Option<OutputVolume>, String>;
    fn set_volume(&self, volume: u8) -> Result<(), String>;
    fn set_muted(&self, muted: bool) -> Result<(), String>;
}

/// A snapshot plus the session that captured it, so a late terminal path from
/// an older recording can never restore (or discard) a newer session's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SavedVolume {
    session: u64,
    state: OutputVolume,
}

/// Per-session volume snapshot and one-shot restoration.
///
/// A single instance lives in app state. Sessions are identified by the
/// recording epoch: the start path calls [`VolumeGuard::begin`], then lowers the
/// volume only after the start cue has finished, and every terminal path
/// restores the same session. `begin` also recovers a snapshot left behind by a
/// session that missed its terminal path. Operations are serialized, so a stop
/// that lands while the start cue is still playing clears the session first and
/// the delayed dim becomes a no-op instead of lowering the volume after cleanup.
pub(crate) struct VolumeGuard {
    controller: Arc<dyn VolumeController>,
    saved: Mutex<Option<SavedVolume>>,
    /// The only session currently allowed to dim/restore. `None` means no
    /// session is active (idle or already restored).
    active: Mutex<Option<u64>>,
    /// Serializes begin/dim/restore so an app-exit cleanup cannot interleave
    /// with a slow start and leave the volume lowered.
    operation: Mutex<()>,
}

impl VolumeGuard {
    pub(crate) fn new(controller: Arc<dyn VolumeController>) -> Self {
        Self {
            controller,
            saved: Mutex::new(None),
            active: Mutex::new(None),
            operation: Mutex::new(()),
        }
    }

    pub(crate) fn new_default() -> Arc<Self> {
        Arc::new(Self::new(default_controller()))
    }

    /// Mark `session` as the active volume owner. Any snapshot left by a
    /// previous session that missed cleanup is restored first, so the new
    /// session always starts from the user's real volume.
    pub(crate) fn begin_session_blocking(&self, session: u64) {
        let _operation = self.operation.lock().unwrap();
        let stale = self.saved.lock().unwrap().take();
        if let Some(stale) = stale {
            log_rec!(
                warn,
                "Found stale system volume state {} from session {}; restoring before session {}",
                stale.state.describe(),
                stale.session,
                session
            );
            self.restore_now(stale.state, "stale session");
        }
        *self.active.lock().unwrap() = Some(session);
    }

    /// Capture the current output state and lower the volume for `session`.
    /// Never returns an error: transcription must proceed even if the volume
    /// operation fails. A dim for a session that is no longer active is ignored
    /// (the most important case: the user stopped while the start cue played).
    pub(crate) fn dim_session_blocking(&self, session: u64) {
        let _operation = self.operation.lock().unwrap();
        if *self.active.lock().unwrap() != Some(session) {
            log_rec!(
                debug,
                "Skipping volume dim for inactive session {}; a newer session or cleanup owns the volume",
                session
            );
            return;
        }

        // Normally empty: `begin_session_blocking` restores stale state. Keep
        // this defensive recovery for a missed cleanup path.
        let stale = self.saved.lock().unwrap().take();
        if let Some(stale) = stale {
            log_rec!(
                warn,
                "Found stale system volume state {}; restoring before dimming",
                stale.state.describe()
            );
            self.restore_now(stale.state, "stale session");
        }

        let original = match self.controller.read() {
            Ok(Some(state)) => state,
            Ok(None) => {
                log_rec!(
                    debug,
                    "System output volume control is unavailable on this platform; skipping"
                );
                return;
            }
            Err(error) => {
                log_rec!(
                    warn,
                    "Could not read system output volume: {}; transcribing without lowering it",
                    error
                );
                return;
            }
        };

        if original.volume <= DIM_VOLUME {
            log_rec!(
                debug,
                "System output volume is already {}; leaving it unchanged",
                original.describe()
            );
            return;
        }

        // Persist the snapshot before mutating anything so any later terminal
        // path can restore it, even if a follow-up mute operation fails.
        *self.saved.lock().unwrap() = Some(SavedVolume {
            session,
            state: original,
        });

        if let Err(error) = self.controller.set_volume(DIM_VOLUME) {
            log_rec!(
                warn,
                "Failed to lower system output volume from {} to {}%: {}; transcribing anyway",
                original.describe(),
                DIM_VOLUME,
                error
            );
            // Nothing was changed; discard the snapshot so cleanup stays a no-op.
            *self.saved.lock().unwrap() = None;
            return;
        }

        // `set volume output volume` unmutes as a side effect, so re-apply the
        // captured muted state when the system started muted.
        if original.muted {
            if let Err(error) = self.controller.set_muted(true) {
                log_rec!(
                    warn,
                    "Lowered system output volume but could not re-apply mute: {}",
                    error
                );
            }
        }

        log_rec!(
            info,
            "Lowered system output volume {} to {}% for transcription (session {})",
            original.describe(),
            DIM_VOLUME,
            session
        );
    }

    /// Restore `session`'s snapshot, at most once. A restore for a session that
    /// is no longer active is ignored; it must never touch a newer session's
    /// snapshot.
    pub(crate) fn restore_session_blocking(&self, session: u64) {
        let _operation = self.operation.lock().unwrap();
        if *self.active.lock().unwrap() != Some(session) {
            log_rec!(
                debug,
                "Ignoring volume restore for inactive session {}",
                session
            );
            return;
        }
        *self.active.lock().unwrap() = None;

        let saved = self.saved.lock().unwrap().take();
        if let Some(saved) = saved {
            if saved.session != session {
                log_rec!(
                    warn,
                    "Volume snapshot belonged to session {} but session {} is ending; restoring it anyway",
                    saved.session,
                    session
                );
            }
            self.restore_now(saved.state, "session end");
        }
    }

    /// Restore whatever snapshot exists, regardless of session. Used by app
    /// exit, where no further session can start.
    pub(crate) fn restore_blocking(&self) {
        let _operation = self.operation.lock().unwrap();
        *self.active.lock().unwrap() = None;
        let saved = self.saved.lock().unwrap().take();
        if let Some(saved) = saved {
            self.restore_now(saved.state, "session end");
        }
    }

    fn restore_now(&self, original: OutputVolume, reason: &str) {
        let volume_result = self.controller.set_volume(original.volume);
        let mute_result = self.controller.set_muted(original.muted);

        match (volume_result, mute_result) {
            (Ok(()), Ok(())) => log_rec!(
                info,
                "Restored system output volume to {} ({})",
                original.describe(),
                reason
            ),
            (volume_result, mute_result) => {
                if let Err(error) = volume_result {
                    log_rec!(
                        error,
                        "Failed to restore system output volume to {} ({}): {}",
                        original.describe(),
                        reason,
                        error
                    );
                }
                if let Err(error) = mute_result {
                    log_rec!(
                        error,
                        "Failed to restore system output mute state to {} ({}): {}",
                        original.muted,
                        reason,
                        error
                    );
                }
            }
        }
    }

    /// Async wrappers keep the blocking `osascript` calls off the async worker
    /// threads.
    pub(crate) async fn begin(self: &Arc<Self>, session: u64) {
        let this = Arc::clone(self);
        if let Err(error) =
            tokio::task::spawn_blocking(move || this.begin_session_blocking(session)).await
        {
            log_rec!(error, "System volume begin task failed: {}", error);
        }
    }

    pub(crate) async fn dim_session(self: &Arc<Self>, session: u64) {
        let this = Arc::clone(self);
        if let Err(error) =
            tokio::task::spawn_blocking(move || this.dim_session_blocking(session)).await
        {
            log_rec!(error, "System volume lower task failed: {}", error);
        }
    }

    pub(crate) async fn restore_session(self: &Arc<Self>, session: u64) {
        let this = Arc::clone(self);
        if let Err(error) =
            tokio::task::spawn_blocking(move || this.restore_session_blocking(session)).await
        {
            log_rec!(error, "System volume restore task failed: {}", error);
        }
    }

    pub(crate) async fn restore(self: &Arc<Self>) {
        let this = Arc::clone(self);
        if let Err(error) = tokio::task::spawn_blocking(move || this.restore_blocking()).await {
            log_rec!(error, "System volume restore task failed: {}", error);
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{OutputVolume, VolumeController};

    /// Absolute path avoids a GUI app's restricted PATH resolving a different
    /// binary. `osascript` ships with macOS (also used by `paste.rs`).
    const OSASCRIPT: &str = "/usr/bin/osascript";
    const SCRIPT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Read output volume and mute state in one AppleScript invocation.
    const READ_SCRIPT: &str = r#"set v to get volume settings
return ((output volume of v) as string) & "|" & ((output muted of v) as string)"#;

    pub(super) struct MacOsVolumeController;

    impl VolumeController for MacOsVolumeController {
        fn read(&self) -> Result<Option<OutputVolume>, String> {
            let output = run_script(READ_SCRIPT)?;
            parse_output(&output).map(Some)
        }

        fn set_volume(&self, volume: u8) -> Result<(), String> {
            let volume = volume.min(100);
            run_script(&format!("set volume output volume {}", volume)).map(|_| ())
        }

        fn set_muted(&self, muted: bool) -> Result<(), String> {
            run_script(&format!(
                "set volume output muted {}",
                if muted { "true" } else { "false" }
            ))
            .map(|_| ())
        }
    }

    /// Parse `"<volume>|<muted>"`, e.g. `"60|false"`.
    fn parse_output(raw: &str) -> Result<OutputVolume, String> {
        let raw = raw.trim();
        let (volume, muted) = raw
            .split_once('|')
            .ok_or_else(|| format!("unexpected volume settings output: {:?}", raw))?;

        let parsed_volume = volume
            .trim()
            .parse::<f64>()
            .map_err(|error| format!("invalid output volume {:?}: {}", volume, error))?;
        if !parsed_volume.is_finite() {
            return Err(format!("invalid output volume {:?}", volume));
        }

        let muted = match muted.trim() {
            "true" => true,
            "false" => false,
            other => return Err(format!("invalid output muted state {:?}", other)),
        };

        Ok(OutputVolume {
            volume: parsed_volume.round().clamp(0.0, 100.0) as u8,
            muted,
        })
    }

    /// Run an AppleScript with a bounded wait so a stuck script cannot pin a
    /// blocking thread forever.
    fn run_script(script: &str) -> Result<String, String> {
        let mut child = Command::new(OSASCRIPT)
            .args(["-e", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to run {}: {}", OSASCRIPT, error))?;

        let deadline = Instant::now() + SCRIPT_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "{} timed out after {}s",
                            OSASCRIPT,
                            SCRIPT_TIMEOUT.as_secs()
                        ));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("failed to wait for {}: {}", OSASCRIPT, error));
                }
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed to read {} output: {}", OSASCRIPT, error))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "{} exited with {}: {}",
                OSASCRIPT,
                output.status,
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_volume_and_mute_state() {
            assert_eq!(
                parse_output("60|false").unwrap(),
                OutputVolume {
                    volume: 60,
                    muted: false
                }
            );
            assert_eq!(
                parse_output("100|true").unwrap(),
                OutputVolume {
                    volume: 100,
                    muted: true
                }
            );
        }

        #[test]
        fn parses_whitespace_and_rounds_fractional_volume() {
            assert_eq!(
                parse_output("  7.6 | true \n").unwrap(),
                OutputVolume {
                    volume: 8,
                    muted: true
                }
            );
        }

        #[test]
        fn clamps_out_of_range_volume() {
            assert_eq!(parse_output("101|false").unwrap().volume, 100);
            assert_eq!(parse_output("-3|false").unwrap().volume, 0);
        }

        #[test]
        fn rejects_malformed_output() {
            assert!(parse_output("").is_err());
            assert!(parse_output("60").is_err());
            assert!(parse_output("loud|false").is_err());
            assert!(parse_output("60|maybe").is_err());
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn default_controller() -> Arc<dyn VolumeController> {
    Arc::new(macos::MacOsVolumeController)
}

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use super::{OutputVolume, VolumeController};

    pub(super) struct UnsupportedVolumeController;

    impl VolumeController for UnsupportedVolumeController {
        fn read(&self) -> Result<Option<OutputVolume>, String> {
            Ok(None)
        }

        fn set_volume(&self, _volume: u8) -> Result<(), String> {
            Ok(())
        }

        fn set_muted(&self, _muted: bool) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn default_controller() -> Arc<dyn VolumeController> {
    Arc::new(unsupported::UnsupportedVolumeController)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct FakeController {
        state: Mutex<OutputVolume>,
        fail_read: AtomicBool,
        fail_next_set_volume: AtomicBool,
        fail_next_set_muted: AtomicBool,
        volume_calls: Mutex<Vec<u8>>,
        mute_calls: Mutex<Vec<bool>>,
    }

    impl FakeController {
        fn new(volume: u8, muted: bool) -> Self {
            Self {
                state: Mutex::new(OutputVolume { volume, muted }),
                fail_read: AtomicBool::new(false),
                fail_next_set_volume: AtomicBool::new(false),
                fail_next_set_muted: AtomicBool::new(false),
                volume_calls: Mutex::new(Vec::new()),
                mute_calls: Mutex::new(Vec::new()),
            }
        }

        fn state(&self) -> OutputVolume {
            *self.state.lock().unwrap()
        }

        fn volume_calls(&self) -> Vec<u8> {
            self.volume_calls.lock().unwrap().clone()
        }

        fn mute_calls(&self) -> Vec<bool> {
            self.mute_calls.lock().unwrap().clone()
        }

        /// Simulate a user changing the system volume while transcription runs.
        fn user_set_volume(&self, volume: u8) {
            let mut state = self.state.lock().unwrap();
            state.volume = volume;
            state.muted = volume == 0;
        }

        fn user_set_muted(&self, muted: bool) {
            self.state.lock().unwrap().muted = muted;
        }
    }

    impl VolumeController for FakeController {
        fn read(&self) -> Result<Option<OutputVolume>, String> {
            if self.fail_read.load(Ordering::SeqCst) {
                return Err("read failed".into());
            }
            Ok(Some(self.state()))
        }

        fn set_volume(&self, volume: u8) -> Result<(), String> {
            self.volume_calls.lock().unwrap().push(volume);
            if self.fail_next_set_volume.swap(false, Ordering::SeqCst) {
                return Err("set volume failed".into());
            }
            // Mimic macOS: setting a volume unmutes, and 0 reports muted.
            let mut state = self.state.lock().unwrap();
            state.volume = volume;
            state.muted = volume == 0;
            Ok(())
        }

        fn set_muted(&self, muted: bool) -> Result<(), String> {
            self.mute_calls.lock().unwrap().push(muted);
            if self.fail_next_set_muted.swap(false, Ordering::SeqCst) {
                return Err("set muted failed".into());
            }
            self.state.lock().unwrap().muted = muted;
            Ok(())
        }
    }

    fn guard(volume: u8, muted: bool) -> (Arc<VolumeGuard>, Arc<FakeController>) {
        let fake = Arc::new(FakeController::new(volume, muted));
        let guard = Arc::new(VolumeGuard::new(fake.clone()));
        (guard, fake)
    }

    /// Start a session and lower the volume, mirroring the lifecycle's
    /// begin → (start cue finishes) → dim ordering.
    fn dim(guard: &VolumeGuard, session: u64) {
        guard.begin_session_blocking(session);
        guard.dim_session_blocking(session);
    }

    fn restore(guard: &VolumeGuard, session: u64) {
        guard.restore_session_blocking(session);
    }

    #[test]
    fn lowers_60_percent_to_10_and_restores_60() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 10,
                muted: false
            }
        );
        restore(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 60,
                muted: false
            }
        );
        assert_eq!(fake.volume_calls(), vec![10, 60]);
    }

    #[test]
    fn lowers_100_percent_to_10_and_restores_100() {
        let (guard, fake) = guard(100, false);
        dim(&guard, 1);
        assert_eq!(fake.state().volume, 10);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 100);
        assert_eq!(fake.volume_calls(), vec![10, 100]);
    }

    #[test]
    fn leaves_10_percent_unchanged() {
        let (guard, fake) = guard(10, false);
        dim(&guard, 1);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 10);
        assert!(fake.volume_calls().is_empty());
        assert!(fake.mute_calls().is_empty());
    }

    #[test]
    fn leaves_5_percent_unchanged() {
        let (guard, fake) = guard(5, false);
        dim(&guard, 1);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 5);
        assert!(fake.volume_calls().is_empty());
    }

    #[test]
    fn leaves_muted_low_volume_unchanged() {
        let (guard, fake) = guard(5, true);
        dim(&guard, 1);
        restore(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 5,
                muted: true
            }
        );
        assert!(fake.volume_calls().is_empty());
        assert!(fake.mute_calls().is_empty());
    }

    #[test]
    fn preserves_and_restores_muted_state() {
        let (guard, fake) = guard(60, true);
        dim(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 10,
                muted: true
            }
        );
        assert_eq!(fake.volume_calls(), vec![10]);
        assert_eq!(fake.mute_calls(), vec![true]);

        restore(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 60,
                muted: true
            }
        );
        assert_eq!(fake.volume_calls(), vec![10, 60]);
        assert_eq!(fake.mute_calls(), vec![true, true]);
    }

    #[test]
    fn restores_saved_volume_after_user_changes_it() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        fake.user_set_volume(30);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 60);
        assert_eq!(fake.volume_calls(), vec![10, 60]);
    }

    #[test]
    fn mute_reapply_failure_keeps_snapshot_for_restore() {
        let (guard, fake) = guard(60, true);
        fake.fail_next_set_muted.store(true, Ordering::SeqCst);
        dim(&guard, 1);
        // The volume was lowered but the mute re-apply failed; the snapshot must
        // survive so terminal cleanup can still restore the original state.
        assert_eq!(fake.state().volume, 10);

        restore(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 60,
                muted: true
            }
        );
        assert_eq!(fake.volume_calls(), vec![10, 60]);
    }

    #[test]
    fn restores_after_successful_completion() {
        let (guard, fake) = guard(80, false);
        dim(&guard, 1);
        // Successful ASR + paste happens here.
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 80);
    }

    #[test]
    fn restores_after_cancellation() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        fake.user_set_muted(true);
        // ESC cancellation happens here.
        restore(&guard, 1);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 60,
                muted: false
            }
        );
    }

    #[test]
    fn restores_after_asr_error() {
        let (guard, fake) = guard(100, false);
        dim(&guard, 1);
        // ASR commit/connection failure happens here.
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 100);
    }

    #[test]
    fn restores_after_post_processing_error() {
        let (guard, fake) = guard(45, false);
        dim(&guard, 1);
        // LLM / clipboard / paste failure happens here.
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 45);
    }

    #[test]
    fn lower_failure_is_non_fatal_and_leaves_no_snapshot() {
        let (guard, fake) = guard(60, false);
        fake.fail_next_set_volume.store(true, Ordering::SeqCst);
        dim(&guard, 1);
        assert_eq!(fake.state().volume, 60);
        assert_eq!(fake.volume_calls(), vec![10]);

        // Cleanup after the failed lower must not mutate anything.
        restore(&guard, 1);
        assert_eq!(fake.volume_calls(), vec![10]);
    }

    #[test]
    fn read_failure_is_non_fatal() {
        let (guard, fake) = guard(60, false);
        fake.fail_read.store(true, Ordering::SeqCst);
        dim(&guard, 1);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 60);
        assert!(fake.volume_calls().is_empty());
    }

    #[test]
    fn restore_failure_is_logged_and_does_not_poison_later_sessions() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        fake.fail_next_set_volume.store(true, Ordering::SeqCst);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 10);
        assert_eq!(fake.volume_calls(), vec![10, 60]);

        // The failed snapshot is consumed, so cleanup is not retried.
        restore(&guard, 1);
        assert_eq!(fake.volume_calls(), vec![10, 60]);

        // A later session still snapshots and restores its own state.
        fake.user_set_volume(75);
        dim(&guard, 1);
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 75);
        assert_eq!(fake.volume_calls(), vec![10, 60, 10, 75]);
    }

    #[test]
    fn duplicate_cleanup_restores_only_once() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        restore(&guard, 1);
        restore(&guard, 1);
        restore(&guard, 1);
        assert_eq!(fake.volume_calls(), vec![10, 60]);
    }

    #[test]
    fn subsequent_session_takes_a_fresh_snapshot() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        restore(&guard, 1);

        fake.user_set_volume(80);
        dim(&guard, 2);
        restore(&guard, 2);

        assert_eq!(fake.volume_calls(), vec![10, 60, 10, 80]);
        assert_eq!(fake.state().volume, 80);
    }

    #[test]
    fn stale_snapshot_is_restored_before_a_new_session_snapshot() {
        let (guard, fake) = guard(60, false);
        // Simulate a missed terminal path: session 1 dims and session 2 begins
        // without session 1 ever being restored.
        dim(&guard, 1);
        dim(&guard, 2);
        restore(&guard, 2);

        // The stale session's original 60% is restored before session 2 takes
        // its own snapshot, so the restoration target is always the user's real
        // volume rather than an already-dimmed value.
        assert_eq!(fake.volume_calls(), vec![10, 60, 10, 60]);
        assert_eq!(fake.state().volume, 60);
    }

    #[test]
    fn delayed_dim_after_stop_is_ignored() {
        // The start sound is still playing when the user stops. Cleanup runs
        // first (no snapshot yet); the deferred dim must not lower the volume
        // afterwards and leave it dimmed with no session to restore it.
        let (guard, fake) = guard(60, false);
        guard.begin_session_blocking(1);
        restore(&guard, 1);
        guard.dim_session_blocking(1);

        assert_eq!(fake.state().volume, 60);
        assert!(fake.volume_calls().is_empty());
    }

    #[test]
    fn late_dim_from_an_old_session_cannot_touch_a_new_session() {
        let (guard, fake) = guard(60, false);
        guard.begin_session_blocking(1);
        guard.begin_session_blocking(2);

        // Session 1's delayed dim arrives after session 2 started.
        guard.dim_session_blocking(1);
        guard.dim_session_blocking(2);

        assert_eq!(fake.state().volume, 10);
        // Only session 2's dim ran.
        assert_eq!(fake.volume_calls(), vec![10]);

        // A late restore from session 1 must not restore session 2's snapshot.
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 10);

        restore(&guard, 2);
        assert_eq!(fake.state().volume, 60);
        assert_eq!(fake.volume_calls(), vec![10, 60]);
    }

    #[test]
    fn restore_then_dim_then_restore_across_repeated_sessions() {
        let (guard, fake) = guard(70, false);
        for session in 1..=3 {
            guard.begin_session_blocking(session);
            guard.dim_session_blocking(session);
            assert_eq!(fake.state().volume, 10);
            restore(&guard, session);
            assert_eq!(fake.state().volume, 70);
        }
        assert_eq!(fake.volume_calls(), vec![10, 70, 10, 70, 10, 70]);
    }
}
