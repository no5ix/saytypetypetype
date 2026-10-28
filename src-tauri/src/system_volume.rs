//! Temporary macOS system output-volume control for a transcription session.
//!
//! On session start we capture the current output volume and mute state, then
//! glide the volume down to [`DIM_VOLUME`] percent over about 400 ms when it
//! is higher. The captured state is restored exactly once when the workflow
//! reaches any terminal outcome — also as a ~400 ms glide — so a user
//! adjustment made while transcribing is intentionally overwritten by the value
//! saved at the start.
//!
//! The lifecycle owns the begin/restore boundaries; this module only owns the
//! per-session snapshot, duplicate-cleanup protection, the ramp cancellation
//! ticket, and the macOS I/O. Volume control is best-effort: a failure is
//! logged and never blocks a transcription, and a failed restore falls back to
//! an immediate exact set so the volume is never left lowered.
//!
//! A ramp is a detached background thread writing one `set volume` step every
//! [`RAMP_STEP`]; its target volume is derived from elapsed wall time rather
//! than a fixed step index, so slow `osascript` round trips shorten the number
//! of steps but never stretch the ~400 ms duration. Every ramp carries a ticket:
//! begin/stop/new-session bumps the ticket and the in-flight ramp aborts at its
//! next check, leaving the volume exactly where the winning operation wants it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Target output volume, in percent, while a transcription session is active.
pub(crate) const DIM_VOLUME: u8 = 10;

/// Total time of a smooth volume glide (dim and restore, each direction).
/// Long enough to read as motion, short enough that the transition never feels
/// like it is holding up the recording workflow.
const RAMP_DURATION: Duration = Duration::from_millis(400);
/// Pause between ramp writes. Small enough to read as continuous motion, large
/// enough not to spam `osascript` (each write spawns a process).
const RAMP_STEP: Duration = Duration::from_millis(30);
/// Safety bound for waiting on a detached ramp thread (it can only overshoot by
/// one step plus its own scheduling delay).
const RAMP_JOIN_SLACK: Duration = Duration::from_millis(250);

/// Eased volume at `progress` (0.0 → `from`, 1.0 → `to`).
///
/// Smoothstep easing: slow at both ends, fastest in the middle, which is what
/// makes the glide read as motion instead of a strobe of slider jumps. The
/// endpoints are exact and intermediate values stay inside `[from, to]`.
fn ramp_volume(from: u8, to: u8, progress: f64) -> u8 {
    let t = progress.clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t); // smoothstep
    let value = f64::from(from) + (f64::from(to) - f64::from(from)) * eased;
    value.round().clamp(0.0, 100.0) as u8
}

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
    /// Serializes begin/dim/restore bookkeeping so an app-exit cleanup cannot
    /// interleave with a slow start and leave the volume lowered. Ramp threads
    /// deliberately run *outside* this lock (see module docs).
    operation: Mutex<()>,
    /// Cancellation ticket for the detached volume ramps. Every begin/dim/
    /// restore takes a fresh ticket; a ramp whose ticket is no longer current
    /// aborts at its next step check, so two ramps never compete.
    ramp_ticket: AtomicU64,
    /// Number of ramp threads that may still write (a count, not a flag: an
    /// aborted ramp dropping out must not clear it while a newer ramp runs).
    ramp_running: AtomicU64,
    /// The volume an in-flight restore ramp is still heading toward. When a new
    /// session begins while that ramp is mid-glide, the raw read would catch a
    /// transient mid-ramp value; the endpoint is the user's real state, so the
    /// next `dim` prefers it over the fresh read. App exit consumes it too: the
    /// snapshot was already taken when the glide was scheduled.
    expected_restore: Mutex<Option<OutputVolume>>,
    /// Volume to snapshot instead of the live read after an interrupted restore
    /// (consumed by the next `dim`).
    user_volume_override: Mutex<Option<u8>>,
    /// Ramp timing in milliseconds (injectable for tests; production uses the
    /// constants). Atomics because the guard lives in an `Arc`.
    ramp_duration_ms: AtomicU64,
    ramp_step_ms: AtomicU64,
}

/// Releases the ramp-thread slot when a spawned ramp thread's body ends, even
/// on panic. The scheduler acquires the slot (increments) before spawning, so a
/// waiter between schedule and thread start never sees "idle".
struct RunningRamp<'a>(&'a VolumeGuard);

impl Drop for RunningRamp<'_> {
    fn drop(&mut self) {
        self.0.ramp_running.fetch_sub(1, Ordering::SeqCst);
    }
}

impl VolumeGuard {
    pub(crate) fn new(controller: Arc<dyn VolumeController>) -> Self {
        Self {
            controller,
            saved: Mutex::new(None),
            active: Mutex::new(None),
            operation: Mutex::new(()),
            ramp_ticket: AtomicU64::new(0),
            ramp_running: AtomicU64::new(0),
            expected_restore: Mutex::new(None),
            user_volume_override: Mutex::new(None),
            ramp_duration_ms: AtomicU64::new(RAMP_DURATION.as_millis() as u64),
            ramp_step_ms: AtomicU64::new(RAMP_STEP.as_millis() as u64),
        }
    }

    /// Override the glide timing (tests use a compressed schedule).
    #[cfg(test)]
    fn set_ramp_timing(&self, duration: Duration, step: Duration) {
        self.ramp_duration_ms
            .store(duration.as_millis() as u64, Ordering::SeqCst);
        self.ramp_step_ms
            .store(step.as_millis() as u64, Ordering::SeqCst);
    }

    fn ramp_duration(&self) -> Duration {
        Duration::from_millis(self.ramp_duration_ms.load(Ordering::SeqCst))
    }

    fn ramp_step(&self) -> Duration {
        Duration::from_millis(self.ramp_step_ms.load(Ordering::SeqCst))
    }

    /// Claim the only ramp slot; any previous ramp aborts at its next check.
    fn take_ramp_ticket(&self) -> u64 {
        self.ramp_ticket.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Invalidate whatever ramp is in flight (session change or cleanup).
    fn cancel_ramps(&self) {
        self.ramp_ticket.fetch_add(1, Ordering::SeqCst);
    }

    fn is_ramp_current(&self, ticket: u64) -> bool {
        self.ramp_ticket.load(Ordering::SeqCst) == ticket
    }

    /// Block (bounded) until no ramp thread is writing.
    fn wait_for_ramp_blocking(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.ramp_running.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Async variant: resolve once any in-flight glide has finished (or the
    /// safety bound elapses). Used to keep the end cue after the restore ramp.
    pub(crate) async fn wait_for_ramp(&self) {
        let budget = self.ramp_duration() * 2 + RAMP_JOIN_SLACK;
        let _ = tokio::time::timeout(budget, async {
            while self.ramp_running.load(Ordering::SeqCst) > 0 {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
        .await;
    }

    /// Glide the output volume from `from` to `to` over `ramp_duration`, one
    /// write per `ramp_step`, aborting as soon as `ticket` is superseded.
    /// Returns whether the ramp reached its endpoint while still current.
    fn run_volume_ramp(&self, from: u8, to: u8, ticket: u64) -> bool {
        if from == to {
            return true;
        }
        let started = Instant::now();
        // Start "at" `from` so the initial value is never written back (macOS
        // would register it as a redundant slider event).
        let mut last_written: Option<u8> = Some(from);
        loop {
            if !self.is_ramp_current(ticket) {
                return false;
            }
            // Target by elapsed time, not step index: slow writes reduce the
            // number of steps but never stretch the duration.
            let progress = started.elapsed().as_secs_f64() / self.ramp_duration().as_secs_f64();
            let value = ramp_volume(from, to, progress);
            if last_written != Some(value) {
                if !self.is_ramp_current(ticket) {
                    return false;
                }
                if let Err(error) = self.controller.set_volume(value) {
                    log_rec!(
                        warn,
                        "Volume ramp aborted at {value}% (target {to}%): {}",
                        error
                    );
                    return false;
                }
                last_written = Some(value);
            }
            if progress >= 1.0 {
                return true;
            }
            std::thread::sleep(
                self.ramp_step()
                    .min(self.ramp_duration().saturating_sub(started.elapsed())),
            );
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
        // This session owns the volume now; an in-flight glide from the
        // previous session must stop immediately.
        self.cancel_ramps();
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
        } else if let Some(state) = self.expected_restore.lock().unwrap().take() {
            // A restore ramp was interrupted mid-glide. The user's real volume
            // is the endpoint it was heading toward, not the mid-ramp value the
            // next snapshot read will see.
            *self.user_volume_override.lock().unwrap() = Some(state.volume);
        }
        *self.active.lock().unwrap() = Some(session);
    }

    /// Capture the current output state and glide the volume down to
    /// [`DIM_VOLUME`] for `session` over ~400 ms. Never returns an error:
    /// transcription must proceed even if the volume operation fails. A dim for
    /// a session that is no longer active is ignored (the most important case:
    /// the user stopped while the start cue played).
    pub(crate) fn dim_session_blocking(self: &Arc<Self>, session: u64) {
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

        let mut original = match self.controller.read() {
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

        // An interrupted restore ramp means the live read caught a mid-glide
        // value. The endpoint it was heading toward is the user's real volume;
        // trust it when it is louder than what we just read. (A manual raise
        // during the same few hundred milliseconds cannot be distinguished —
        // same intent-override rule the snapshot restore already applies.)
        if let Some(volume) = self.user_volume_override.lock().unwrap().take() {
            if volume > original.volume {
                log_rec!(
                    debug,
                    "Using {}% as the session start volume: a restore ramp was interrupted at {}%",
                    volume,
                    original.volume
                );
                original.volume = volume;
            }
        }

        if original.volume <= DIM_VOLUME {
            log_rec!(
                debug,
                "System output volume is already {}; leaving it unchanged",
                original.describe()
            );
            return;
        }

        // Persist the snapshot before mutating anything so any later terminal
        // path can restore it, even if a follow-up operation fails.
        *self.saved.lock().unwrap() = Some(SavedVolume {
            session,
            state: original,
        });

        if original.muted {
            // A muted session cannot hear a glide, and `set volume` unmutes as a
            // side effect, so jump straight to the target and re-apply the mute.
            if let Err(error) = self.controller.set_volume(DIM_VOLUME) {
                log_rec!(
                    warn,
                    "Failed to lower system output volume from {} to {}%: {}; transcribing anyway",
                    original.describe(),
                    DIM_VOLUME,
                    error
                );
                *self.saved.lock().unwrap() = None;
                return;
            }
            if let Err(error) = self.controller.set_muted(true) {
                log_rec!(
                    warn,
                    "Lowered system output volume but could not re-apply mute: {}",
                    error
                );
            }
            log_rec!(
                info,
                "Lowered system output volume {} to {}% for transcription (session {})",
                original.describe(),
                DIM_VOLUME,
                session
            );
            return;
        }

        // Smooth dimming on a detached thread: the recording pipeline never
        // waits on it, and the guard ticket cancels it the moment this session
        // stops or a newer session takes over.
        let ticket = self.take_ramp_ticket();
        self.ramp_running.fetch_add(1, Ordering::SeqCst);
        let this = Arc::clone(self);
        let from = original.volume;
        let spawned = std::thread::Builder::new()
            .name("voicepaste-vol-dim".to_string())
            .spawn(move || {
                let _running = RunningRamp(&this);
                let completed = this.run_volume_ramp(from, DIM_VOLUME, ticket);
                if !this.is_ramp_current(ticket) {
                    return; // superseded; the new owner sets the volume
                }
                if completed {
                    log_rec!(
                        info,
                        "Lowered system output volume {} to {}% for transcription (session {})",
                        original.describe(),
                        DIM_VOLUME,
                        session
                    );
                } else {
                    // A step failed mid-glide: undo the partial dimming so the
                    // user hears their own volume again, and drop the snapshot
                    // (nothing net changed).
                    *this.saved.lock().unwrap() = None;
                    let _ = this.controller.set_volume(from);
                    log_rec!(
                        warn,
                        "Could not finish lowering system output volume from {}; transcribing at it anyway",
                        original.describe()
                    );
                }
            });
        if spawned.is_err() {
            self.ramp_running.fetch_sub(1, Ordering::SeqCst);
            if let Err(error) = self.controller.set_volume(DIM_VOLUME) {
                log_rec!(
                    warn,
                    "Failed to lower system output volume from {} to {}%: {}; transcribing anyway",
                    original.describe(),
                    DIM_VOLUME,
                    error
                );
                *self.saved.lock().unwrap() = None;
            }
        }
    }

    /// Restore `session`'s snapshot, at most once, gliding back to the original
    /// volume over ~400 ms on a detached thread (callers never block on it). A
    /// restore for a session that is no longer active is ignored; it must never
    /// touch a newer session's snapshot.
    pub(crate) fn restore_session_blocking(self: &Arc<Self>, session: u64) {
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
        self.cancel_ramps();

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
            self.restore_state_blocking(saved.state, "session end");
        }
    }

    /// Restore whatever snapshot exists, synchronously and exactly. Used by app
    /// exit, where no further session can start and no detached thread may be
    /// left behind mid-glide.
    pub(crate) fn restore_blocking(self: &Arc<Self>) {
        let _operation = self.operation.lock().unwrap();
        *self.active.lock().unwrap() = None;
        // Cancel first so an in-flight ramp stops writing, then wait for its
        // last write to land before the exact restore below — otherwise a step
        // could still be in flight and overwrite the final value.
        self.cancel_ramps();
        self.wait_for_ramp_blocking(self.ramp_step() + RAMP_JOIN_SLACK);
        let interrupted = self.expected_restore.lock().unwrap().take();
        let saved = self.saved.lock().unwrap().take();
        if let Some(saved) = saved {
            self.restore_now(saved.state, "session end");
        } else if let Some(state) = interrupted {
            // A restore glide was scheduled but cancelled mid-flight: land the
            // endpoint it was heading toward so shutdown never leaves the
            // volume lowered.
            self.restore_now(state, "interrupted restore glide");
        }
    }

    /// Schedule an async restore of `state`: an audible smooth glide when the
    /// session was unmuted, an exact immediate set otherwise (a muted user must
    /// not hear a leak while `set volume` unmutes mid-ramp).
    fn restore_state_blocking(self: &Arc<Self>, state: OutputVolume, reason: &'static str) {
        if state.muted {
            self.restore_now(state, reason);
            return;
        }
        let from = match self.controller.read() {
            Ok(Some(current)) => current.volume,
            _ => {
                self.restore_now(state, reason);
                return;
            }
        };
        if from == state.volume {
            self.restore_now(state, reason);
            return;
        }

        let ticket = self.take_ramp_ticket();
        *self.expected_restore.lock().unwrap() = Some(state);
        self.ramp_running.fetch_add(1, Ordering::SeqCst);
        let this = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("voicepaste-vol-restore".to_string())
            .spawn(move || {
                let _running = RunningRamp(&this);
                let completed = this.run_volume_ramp(from, state.volume, ticket);
                if !this.is_ramp_current(ticket) {
                    // A newer session or cleanup owns the volume now. `begin`
                    // consumes the endpoint for its own snapshot; `restore_
                    // blocking` lands it exactly on shutdown.
                    return;
                }
                if !completed {
                    log_rec!(
                        warn,
                        "Volume restore glide to {} did not finish cleanly; setting it directly",
                        state.describe()
                    );
                }
                // Land the exact volume and mute state regardless of where the
                // glide stopped, so the session never leaves the volume reduced.
                *this.expected_restore.lock().unwrap() = None;
                this.restore_now(state, reason);
            });
        if spawned.is_err() {
            self.ramp_running.fetch_sub(1, Ordering::SeqCst);
            *self.expected_restore.lock().unwrap() = None;
            self.restore_now(state, reason);
        }
    }

    fn restore_now(&self, original: OutputVolume, reason: &str) {
        // The volume is exactly right after this, so an interrupt override from
        // an earlier interrupted glide is stale.
        *self.user_volume_override.lock().unwrap() = None;
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

    /// Test ramp timing: short enough to keep the suite fast, long enough that
    /// a dozen writes land per glide (the production budget is 400ms/30ms).
    const TEST_RAMP: Duration = Duration::from_millis(120);
    const TEST_STEP: Duration = Duration::from_millis(12);

    fn guard(volume: u8, muted: bool) -> (Arc<VolumeGuard>, Arc<FakeController>) {
        let fake = Arc::new(FakeController::new(volume, muted));
        let guard = Arc::new(VolumeGuard::new(fake.clone()));
        guard.set_ramp_timing(TEST_RAMP, TEST_STEP);
        (guard, fake)
    }

    /// Block until the detached ramp thread finishes (bounded).
    fn settle(guard: &VolumeGuard) {
        guard.wait_for_ramp_blocking(TEST_RAMP + Duration::from_millis(500));
    }

    /// Start a session and glide the volume down, mirroring the lifecycle's
    /// begin → (start cue finishes) → dim ordering.
    fn dim(guard: &Arc<VolumeGuard>, session: u64) {
        guard.begin_session_blocking(session);
        guard.dim_session_blocking(session);
        settle(guard);
    }

    fn restore(guard: &Arc<VolumeGuard>, session: u64) {
        guard.restore_session_blocking(session);
        settle(guard);
    }

    /// The dim glide is strictly descending and lands exactly on `to`.
    fn assert_descending(calls: &[u8], to: u8) {
        assert!(calls.len() >= 3, "glide too coarse: {calls:?}");
        assert!(
            calls.windows(2).all(|w| w[0] > w[1]),
            "not strictly descending: {calls:?}"
        );
        assert_eq!(*calls.last().unwrap(), to, "glide must land on {to}");
    }

    /// The restore glide is ascending (allowing the final exact-set duplicate)
    /// and lands exactly on `to`.
    fn assert_ascending(calls: &[u8], to: u8) {
        assert!(calls.len() >= 3, "glide too coarse: {calls:?}");
        assert!(
            calls.windows(2).all(|w| w[0] <= w[1]),
            "not ascending: {calls:?}"
        );
        assert!(
            calls.windows(2).any(|w| w[0] < w[1]),
            "no motion in glide: {calls:?}"
        );
        assert_eq!(*calls.last().unwrap(), to, "glide must land on {to}");
    }

    /// Run an async guard helper without tauri's runtime (tokio is available).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    #[test]
    fn ramp_volume_interpolation_endpoints_and_monotonicity() {
        // Exact endpoints in both directions.
        assert_eq!(ramp_volume(80, 5, 0.0), 80);
        assert_eq!(ramp_volume(80, 5, 1.0), 5);
        assert_eq!(ramp_volume(5, 80, 0.0), 5);
        assert_eq!(ramp_volume(5, 80, 1.0), 80);
        // Progress outside [0,1] clamps to the endpoints.
        assert_eq!(ramp_volume(80, 5, -0.5), 80);
        assert_eq!(ramp_volume(80, 5, 1.5), 5);
        // A no-op range never overshoots.
        assert_eq!(ramp_volume(40, 40, 0.5), 40);

        // Downward sweep stays inside the range, moves smoothly (no jump over
        // five points between steps) and is monotonic.
        let mut previous = 80;
        let mut steps = 0;
        for i in 0..=100 {
            let value = ramp_volume(80, 5, i as f64 / 100.0);
            assert!((5..=80).contains(&value), "progress {i} -> {value}");
            assert!(value <= previous);
            assert!(
                previous - value <= 5,
                "jump too large at {i}: {previous}->{value}"
            );
            if value != previous {
                steps += 1;
            }
            previous = value;
        }
        assert!(
            steps >= 20,
            "interpolation too coarse: {steps} distinct steps"
        );

        // Smoothstep: slow near the ends, fastest mid-glide.
        assert!(
            (80 - ramp_volume(80, 5, 0.1)) <= 3,
            "start must ease out gently"
        );
        assert!(
            (ramp_volume(5, 80, 0.55) - ramp_volume(5, 80, 0.45)) >= 6,
            "mid-glide must move fastest"
        );
    }

    #[test]
    fn ramp_timing_constants_stay_in_the_desired_window() {
        // ~400 ms per direction, ~25–60 ms steps: fast but still a glide.
        assert!((300..=550).contains(&RAMP_DURATION.as_millis()));
        assert!((25..=60).contains(&RAMP_STEP.as_millis()));
    }

    #[test]
    fn lowers_60_percent_to_10_and_restores_60() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        assert_descending(&fake.volume_calls(), DIM_VOLUME);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 10,
                muted: false
            }
        );
        let after_dim = fake.volume_calls().len();
        restore(&guard, 1);
        assert_ascending(&fake.volume_calls()[after_dim..], 60);
        assert_eq!(
            fake.state(),
            OutputVolume {
                volume: 60,
                muted: false
            }
        );
    }

    #[test]
    fn dim_glides_in_small_incremental_steps() {
        let (guard, fake) = guard(100, false);
        dim(&guard, 1);
        let calls = fake.volume_calls();
        // The first write already moved off the original (no redundant set),
        // and the motion is gradual. Granularity scales with the injected
        // schedule: with production timing (400ms / 30ms ≈ 13 writes over a
        // 90-point range) no step jumps more than ~12%, while smoothstep's
        // 1.5× mid-glide speed bounds any gap by 2× the average step here.
        assert!(calls[0] < 100);
        assert!(calls.len() >= 5, "glide too coarse: {calls:?}");
        let range = 100 - u32::from(DIM_VOLUME);
        let max_gap = (range * 2 / calls.len() as u32) as i32;
        assert!(
            calls
                .windows(2)
                .all(|w| i32::from(w[0]) - i32::from(w[1]) <= max_gap),
            "steps too large: {calls:?}"
        );
        assert_eq!(*calls.last().unwrap(), DIM_VOLUME);
    }

    #[test]
    fn lowers_100_percent_to_10_and_restores_100() {
        let (guard, fake) = guard(100, false);
        dim(&guard, 1);
        assert_eq!(fake.state().volume, 10);
        let after_dim = fake.volume_calls().len();
        restore(&guard, 1);
        assert_ascending(&fake.volume_calls()[after_dim..], 100);
        assert_eq!(fake.state().volume, 100);
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
        // Muted sessions skip the inaudible glide and jump exactly, so the
        // `set volume` unmute side effect never leaks sound.
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
        assert_eq!(*fake.volume_calls().last().unwrap(), 60);
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
        assert_eq!(fake.mute_calls(), vec![true, true]);
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
        // The first glide step failed; the ramp undoes its partial work and the
        // user keeps their original volume.
        assert_eq!(fake.state().volume, 60);
        let calls = fake.volume_calls();
        assert_eq!(*calls.last().unwrap(), 60, "partial dim must be undone");

        // Cleanup after the failed lower must not mutate anything.
        restore(&guard, 1);
        assert_eq!(fake.volume_calls(), calls);
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
    fn restore_failure_falls_back_to_exact_set_and_does_not_poison_later_sessions() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        fake.fail_next_set_volume.store(true, Ordering::SeqCst);
        restore(&guard, 1);
        // A failed glide step must still leave the exact original volume —
        // the terminal path never exits with the volume lowered.
        assert_eq!(fake.state().volume, 60);
        assert_eq!(*fake.volume_calls().last().unwrap(), 60);

        // The snapshot is consumed, so duplicate cleanup is a no-op.
        let calls = fake.volume_calls();
        restore(&guard, 1);
        assert_eq!(fake.volume_calls(), calls);

        // A later session still snapshots and restores its own state.
        fake.user_set_volume(75);
        dim(&guard, 2);
        assert_eq!(fake.state().volume, 10);
        restore(&guard, 2);
        assert_eq!(fake.state().volume, 75);
        assert_eq!(*fake.volume_calls().last().unwrap(), 75);
    }

    #[test]
    fn duplicate_cleanup_restores_only_once() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        restore(&guard, 1);
        let calls = fake.volume_calls();
        assert_eq!(*calls.last().unwrap(), 60);
        restore(&guard, 1);
        restore(&guard, 1);
        assert_eq!(fake.volume_calls(), calls);
    }

    #[test]
    fn subsequent_session_takes_a_fresh_snapshot() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        restore(&guard, 1);

        fake.user_set_volume(80);
        dim(&guard, 2);
        assert_eq!(fake.state().volume, 10);
        restore(&guard, 2);

        assert_eq!(fake.state().volume, 80);
        assert_eq!(*fake.volume_calls().last().unwrap(), 80);
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
        let calls = fake.volume_calls();
        assert!(
            calls.windows(2).any(|w| w == [10, 60]),
            "stale restore must jump back to 60 before the next dim: {calls:?}"
        );
        assert_eq!(fake.state().volume, 60);
        assert_eq!(*calls.last().unwrap(), 60);
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
        settle(&guard);

        assert_eq!(fake.state().volume, 10);
        // Only session 2's dim ran.
        assert_descending(&fake.volume_calls(), DIM_VOLUME);

        // A late restore from session 1 must not restore session 2's snapshot.
        restore(&guard, 1);
        assert_eq!(fake.state().volume, 10);

        restore(&guard, 2);
        assert_eq!(fake.state().volume, 60);
        assert_eq!(*fake.volume_calls().last().unwrap(), 60);
    }

    #[test]
    fn stop_during_dim_ramp_aborts_it_and_restores_exactly() {
        let (guard, fake) = guard(60, false);
        guard.begin_session_blocking(1);
        guard.set_ramp_timing(Duration::from_millis(400), TEST_STEP);
        guard.dim_session_blocking(1);
        // Catch the glide mid-descent, then stop the session.
        std::thread::sleep(Duration::from_millis(60));
        restore(&guard, 1);

        let calls = fake.volume_calls();
        assert!(
            !calls.contains(&DIM_VOLUME),
            "aborted dim must never reach the dim target: {calls:?}"
        );
        assert_eq!(fake.state().volume, 60);
    }

    #[test]
    fn new_session_during_restore_ramp_snapshots_the_intended_volume() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        // Schedule a long restore glide and start a new session mid-flight.
        guard.set_ramp_timing(Duration::from_millis(400), TEST_STEP);
        guard.restore_session_blocking(1);
        std::thread::sleep(Duration::from_millis(60));
        let before_session_two = fake.volume_calls().len();
        guard.begin_session_blocking(2);
        guard.dim_session_blocking(2);
        settle(&guard);

        // Without the endpoint override, session 2 would have read the mid-glide
        // value (~15%) and restored to that instead of the user's real volume.
        let calls = fake.volume_calls();
        assert!(
            calls[before_session_two] > 40,
            "session 2 must dim from ~60%, not from the mid-ramp value: {calls:?}"
        );
        assert_eq!(fake.state().volume, 10);
        restore(&guard, 2);
        assert_eq!(fake.state().volume, 60);
        assert_eq!(*fake.volume_calls().last().unwrap(), 60);
    }

    #[test]
    fn restore_then_dim_then_restore_across_repeated_sessions() {
        let (guard, fake) = guard(70, false);
        for session in 1..=3 {
            guard.begin_session_blocking(session);
            guard.dim_session_blocking(session);
            settle(&guard);
            assert_eq!(fake.state().volume, 10);
            restore(&guard, session);
            assert_eq!(fake.state().volume, 70);
            assert_eq!(*fake.volume_calls().last().unwrap(), 70);
        }
    }

    #[test]
    fn shutdown_restore_lands_exact_volume_over_in_flight_ramp() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        // Schedule a slow restore glide, then quit the app mid-glide.
        guard.set_ramp_timing(Duration::from_millis(400), TEST_STEP);
        guard.restore_session_blocking(1);
        std::thread::sleep(Duration::from_millis(40));
        guard.restore_blocking();

        // The synchronous exit path cancels the ramp and lands the exact value
        // before returning — the volume is never left lowered after shutdown.
        assert_eq!(fake.state().volume, 60);
        assert_eq!(*fake.volume_calls().last().unwrap(), 60);
    }

    #[test]
    fn wait_for_ramp_returns_after_the_glide_lands() {
        let (guard, fake) = guard(60, false);
        dim(&guard, 1);
        guard.restore_session_blocking(1);
        // The end cue waits on this before playing; when it resolves, the exact
        // original volume must already be in place.
        block_on(guard.wait_for_ramp());
        assert_eq!(fake.state().volume, 60);
        assert_eq!(guard.ramp_running.load(Ordering::SeqCst), 0);
    }
}
