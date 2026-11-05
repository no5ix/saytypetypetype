//! ASR memory profiling harness (macOS).
//!
//! Reproduces the first-dictation memory jump in a repeatable, instrumented
//! way: loads the SAME recognizer the app uses for the configured local model
//! (plus the punctuation model and the Silero VAD), then replays a bundled
//! speech fixture through the exact simulated-streaming session the app runs,
//! sampling resident set size (RSS) before the load, after each model, at the
//! decode peak, and after everything is released.
//!
//! It exists so "dictation makes the app jump from ~43 MB to ~1 GB" can be
//! answered with numbers instead of guesses: model bytes on disk vs. RSS after
//! load vs. transient decode peak.
//!
//! Gated behind `asr-integration` AND `#[ignore]` so it never runs in CI or in
//! the normal `cargo test` pass. It downloads nothing; it skips (with a
//! notice) when the model is not present in an app-data directory.
//!
//! ```text
//! # production or dev bundle data dir, whichever has the model downloaded:
//! cargo test --release --features asr-integration asr_memory -- \
//!   --ignored --nocapture
//!
//! # knobs (all optional):
//! #   VOICEPASTE_MEMORY_MODEL_ID   registry id (default: first downloaded parakeet)
//! #   VOICEPASTE_MEMORY_THREADS    ORT threads (default: 8, matching the
//! #                                reported user profile: num_threads: 8)
//! #   VOICEPASTE_MEMORY_SPEED      playback multiplier (default: 4)
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::asr::sherpa_onnx::nemo_transducer::build_nemo_transducer_recognizer;
use crate::asr::sherpa_onnx::punct::PunctuationProcessor;
use crate::asr::sherpa_onnx::simulated_streaming::spawn_simulated_streaming_worker;
use crate::asr::sherpa_onnx::vad::{VadConfig, VadProcessor};
use crate::asr::AsrSession;
use crate::model::{ModelEntry, ModelRegistry};

/// Current resident set size of this process, in bytes (macOS
/// `task_info(MACH_TASK_BASIC_INFO)` — the same field Activity Monitor's
/// "Memory" column is derived from).
fn rss_bytes() -> u64 {
    // libc keeps mach_task_self behind a deprecation pointer to mach2; the
    // function itself is stable C ABI, and pulling in another dep for one
    // profiler call is not worth it.
    #![allow(deprecated)]
    unsafe {
        let mut info: libc::mach_task_basic_info = std::mem::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        let code = libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            &mut info as *mut libc::mach_task_basic_info as libc::task_info_t,
            &mut count,
        );
        if code == libc::KERN_SUCCESS {
            info.resident_size
        } else {
            0
        }
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/tests/fixtures")
        .join(name)
}

/// App-data candidates for both bundle identifiers, dev first (it is the one
/// the reported profile uses).
fn data_dirs() -> Vec<PathBuf> {
    let base = dirs::data_dir().unwrap_or_default();
    vec![
        base.join("com.yolanda.voicepaste.dev"),
        base.join("com.yolanda.voicepaste"),
    ]
}

fn registry() -> ModelRegistry {
    serde_json::from_str(include_str!("../../../registry.json")).expect("registry.json parses")
}

/// Resolve the parakeet entry to profile: explicit id, else the first parakeet
/// directory actually downloaded under a candidate data dir.
fn resolve_model(reg: &ModelRegistry) -> Option<(ModelEntry, PathBuf)> {
    let wanted = std::env::var("VOICEPASTE_MEMORY_MODEL_ID").ok();
    for dir in data_dirs() {
        let models = dir.join("models");
        let Ok(read) = std::fs::read_dir(&models) else {
            continue;
        };
        let mut ids: Vec<String> = read
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|name| name.starts_with("sherpa-onnx-nemo-parakeet"))
            .collect();
        ids.sort();
        if let Some(id) = wanted.clone() {
            ids.retain(|name| name == &id);
        }
        for id in ids {
            let Some(entry) = reg.models.iter().find(|m| m.id == id) else {
                continue;
            };
            let complete = entry
                .model_files
                .values()
                .all(|filename| models.join(&id).join(filename).exists());
            if complete {
                return Some((entry.clone(), models.join(&id)));
            }
        }
    }
    None
}

/// Total bytes of the model files the entry references (disk footprint — the
/// number users see next to a download, NOT a runtime allocation).
fn model_file_bytes(entry: &ModelEntry, model_dir: &Path) -> u64 {
    entry
        .model_files
        .values()
        .filter_map(|filename| std::fs::metadata(model_dir.join(filename)).ok())
        .map(|meta| meta.len())
        .sum()
}

/// Read a 16-bit mono WAV into f32 samples (same shape the app feeds ASR).
fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let data_pos = data
        .windows(4)
        .position(|w| w == b"data")
        .ok_or_else(|| "data chunk not found in WAV".to_string())?;
    let size = u32::from_le_bytes(
        data[data_pos + 4..data_pos + 8]
            .try_into()
            .map_err(|_| "malformed WAV data chunk".to_string())?,
    ) as usize;
    let start = data_pos + 8;
    let end = (start + size).min(data.len());
    Ok(data[start..end]
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}

/// Sample RSS from a background thread and return the peak, dropping the
/// sampler when its sender half is dropped.
fn start_peak_sampler() -> (
    Arc<std::sync::atomic::AtomicU64>,
    std::sync::mpsc::Sender<()>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let peak = Arc::new(AtomicU64::new(0));
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let sampler_peak = Arc::clone(&peak);
    std::thread::spawn(move || loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        let value = rss_bytes();
        sampler_peak.fetch_max(value, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(50));
    });
    (peak, stop_tx)
}

/// Reproduce the first dictation session end to end and report the memory at
/// each step. Run with `--ignored --nocapture`; see the module docs.
#[test]
#[ignore = "heavy: loads the real multi-hundred-MB ASR model; run manually for profiling"]
fn asr_model_memory_profile() {
    let reg = registry();
    let Some((entry, model_dir)) = resolve_model(&reg) else {
        eprintln!("SKIP: no downloaded parakeet model found under app data dirs");
        return;
    };
    let threads: u32 = std::env::var("VOICEPASTE_MEMORY_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let speed: f64 = std::env::var("VOICEPASTE_MEMORY_SPEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &f64| *v > 0.0)
        .unwrap_or(4.0);

    let disk_bytes = model_file_bytes(&entry, &model_dir);
    let rss0 = rss_bytes();
    eprintln!("== ASR memory profile ==");
    eprintln!(
        "model={} threads={} disk_files={:.0} MB rss_before={:.0} MB",
        entry.id,
        threads,
        mb(disk_bytes),
        mb(rss0)
    );

    // Step 1 — recognizer load (what the first dictation pays for).
    let started = Instant::now();
    let recognizer = build_nemo_transducer_recognizer(
        &model_dir,
        &entry,
        threads,
        &serde_json::json!({ "provider": "cpu" }),
    )
    .expect("recognizer should load");
    let recognizer = Arc::new(recognizer);
    let rss1 = rss_bytes();
    eprintln!(
        "after recognizer load: +{:.0} MB (total {:.0} MB, {} ms)",
        mb(rss1 - rss0),
        mb(rss1),
        started.elapsed().as_millis()
    );

    // Step 2 — the satellites every sherpa session creates: VAD + punctuation.
    let data_dir = model_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let vad_entry = reg
        .models
        .iter()
        .find(|m| m.id == "silero-vad")
        .expect("silero-vad in registry");
    let vad = VadProcessor::new(
        vad_entry,
        &data_dir.join("models").join("silero-vad"),
        &VadConfig::default(),
    )
    .expect("vad model");

    let punct_entry = reg
        .models
        .iter()
        .find(|m| m.category == crate::model::ModelCategory::Punctuation)
        .expect("punctuation entry in registry");
    let punct_dir = data_dir.join("models").join(&punct_entry.id);
    let punct = PunctuationProcessor::new(punct_entry, &punct_dir, 2, "cpu")
        .map(Arc::new)
        .ok();
    let rss2 = rss_bytes();
    eprintln!(
        "after vad{}: +{:.0} MB (total {:.0} MB)",
        if punct.is_some() {
            " + punctuation"
        } else {
            ""
        },
        mb(rss2 - rss1),
        mb(rss2)
    );
    drop(vad);
    drop(punct);

    // Step 3 — a live-shaped session: VAD + interim decodes + segment decodes
    // fed at capture cadence, sampling the decode peak.
    let samples = read_wav(&fixture("test-audio-long.wav")).expect("fixture wav");
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
    let vad_for_session = VadProcessor::new(
        vad_entry,
        &data_dir.join("models").join("silero-vad"),
        &VadConfig::default(),
    )
    .expect("vad model");
    let (session, _tx_owner) =
        spawn_simulated_streaming_worker(Arc::clone(&recognizer), vad_for_session, event_tx, None)
            .expect("worker spawns");

    let (peak, stop_tx) = start_peak_sampler();
    let chunk = 1600usize; // 100 ms at 16 kHz — the app's capture chunk size
    for piece in samples.chunks(chunk) {
        session.append_audio(piece);
        std::thread::sleep(Duration::from_secs_f64(0.1 / speed));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let final_text = rt
        .block_on(session.commit_and_await_final())
        .expect("final transcript");
    stop_tx.send(()).ok();
    let rss3 = rss_bytes();
    eprintln!(
        "after {} s session: total {:.0} MB, decode peak {:.0} MB (text {} chars)",
        samples.len() as f64 / 16000.0,
        mb(rss3),
        mb(peak.load(std::sync::atomic::Ordering::Relaxed)),
        final_text.chars().count()
    );

    // Step 4 — release everything: what actually goes back to the OS.
    drop(recognizer);
    std::thread::sleep(Duration::from_millis(300));
    let rss4 = rss_bytes();
    eprintln!(
        "after releasing all models: total {:.0} MB (freed {:.0} MB)",
        mb(rss4),
        mb(rss3.saturating_sub(rss4))
    );

    assert!(
        rss1 > rss0 + 100 * 1024 * 1024,
        "recognizer load should grow RSS by well over 100 MB, grew {:.0} MB",
        mb(rss1 - rss0)
    );
    assert!(
        !final_text.trim().is_empty(),
        "the session must still transcribe the fixture"
    );
}

/// App state scaffolding without a Tauri window: temp data dirs only.
fn temp_app_inner() -> std::sync::Arc<crate::app_state::AppInner> {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_manager = crate::config::ConfigManager::new(dir.path(), dir.path());
    let hotword_manager = crate::hotword::HotwordManager::new(dir.path(), dir.path());
    let stats = crate::stats::StatsService::new(dir.path());
    // Keep the temp dir alive for the lifetime of the process during tests by
    // leaking it; the harness only needs valid paths.
    std::mem::forget(dir);
    crate::app_state::create_app_state(
        config_manager,
        hotword_manager,
        std::path::PathBuf::from("test.log"),
        stats,
    )
}

/// Load the real recognizer into a fresh app state's model cache.
fn load_real_recognizer(
    inner: &std::sync::Arc<crate::app_state::AppInner>,
    entry: &ModelEntry,
    model_dir: &Path,
) -> Arc<sherpa_onnx::OfflineRecognizer> {
    let (recognizer, loaded) = inner
        .asr_models
        .offline_slot()
        .get_or_load("residency-parakeet", || {
            build_nemo_transducer_recognizer(
                model_dir,
                entry,
                2,
                &serde_json::json!({ "provider": "cpu" }),
            )
            .map(Arc::new)
        })
        .expect("recognizer loads");
    assert!(loaded, "first access must load the recognizer");
    recognizer
}

/// ASR model residency (product requirement): once a recognizer is loaded it
/// must stay cached for the rest of the application session, so every later
/// dictation starts warm. Nothing may unload it for being idle — no 5- or
/// 10-minute timer, no state-machine hook. This test loads the real model,
/// lets it idle past a typical "5 minute" profiling window, then requires the
/// warm cache to answer the next session with the *same* recognizer instance.
///
/// The final phase documents the only sanctioned release path: an explicit
/// `unload` (app exit; ASR config, model, or hotword change).
#[test]
#[ignore = "heavy: loads the real multi-hundred-MB ASR model"]
fn asr_cache_stays_loaded_across_idle_windows() {
    let reg = registry();
    let Some((entry, model_dir)) = resolve_model(&reg) else {
        eprintln!("SKIP: no downloaded parakeet model found");
        return;
    };

    let inner = temp_app_inner();
    let recognizer = load_real_recognizer(&inner, &entry, &model_dir);
    let rss_loaded = rss_bytes();

    // Idle past the 5-minute window an earlier design used to release the
    // model. Production waits forever: the whole point is that this sleep
    // changes nothing. (The env override VOICEPASTE_ASR_IDLE_UNLOAD_SECONDS
    // existed only for that design and is gone; 310 s here proves that no
    // timer of that class fires.)
    let idle = std::time::Duration::from_secs(310);
    eprintln!(
        "residency: model loaded, RSS {:.0} MB, idling {} s",
        mb(rss_loaded),
        idle.as_secs()
    );
    std::thread::sleep(idle);

    assert!(
        inner.asr_models.offline_slot().is_loaded(),
        "the recognizer must remain cached while the app idles"
    );

    // The next dictation must be a warm hit: a cache miss would run the
    // loader closure, and that closure is `unreachable!()` on purpose.
    let (warm, loaded_again) = inner
        .asr_models
        .offline_slot()
        .get_or_load("residency-parakeet", || {
            unreachable!("the cached recognizer must be reused, never reloaded")
        })
        .expect("warm cache hit");
    assert!(!loaded_again, "idle time must not force a model reload");
    assert!(
        Arc::ptr_eq(&recognizer, &warm),
        "the warm cache must hand out the same recognizer instance"
    );

    // RSS keeps its plateau: the model is still resident.
    let rss_after = rss_bytes();
    eprintln!("residency: RSS {:.0} MB after idle", mb(rss_after));
    assert!(
        rss_after + 50 * 1024 * 1024 > rss_loaded,
        "idling must not release the model's resident memory"
    );

    // Explicit release is the only way the cache empties.
    drop(warm);
    drop(recognizer);
    inner.asr_models.unload("app exit");
    assert!(
        !inner.asr_models.offline_slot().is_loaded(),
        "an explicit unload releases the recognizer"
    );
}
