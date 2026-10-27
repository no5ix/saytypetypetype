//! Parakeet TDT smoke test and latency benchmark.
//!
//! Uses the real registry entry + the same builder the app uses, so a pass here
//! proves the shipped integration (registry → model files → sherpa-onnx
//! `nemo_transducer`) is functional, not just that sherpa-onnx exists.
//!
//! Gated behind `asr-integration`; skips (with a notice) when the model is not
//! downloaded. No test downloads anything.
//!
//! ```text
//! # after downloading the model in the app (or pointing at a directory):
//! VOICEPASTE_PARAKEET_MODEL_DIR="$TMPDIR/…/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8" \
//!   cargo test --release --features asr-integration parakeet -- --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::asr::sherpa_onnx::nemo_transducer::build_nemo_transducer_recognizer;
use crate::model::{ModelEntry, ModelRegistry};

const DEFAULT_MODEL_ID: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";

/// Registry entry for the model under test (default: Parakeet TDT v3 int8,
/// override with `VOICEPASTE_PARAKEET_MODEL_ID`).
fn registry_entry() -> ModelEntry {
    let model_id =
        std::env::var("VOICEPASTE_PARAKEET_MODEL_ID").unwrap_or_else(|_| DEFAULT_MODEL_ID.into());
    let registry: ModelRegistry =
        serde_json::from_str(include_str!("../../../registry.json")).expect("registry.json parses");
    registry
        .models
        .into_iter()
        .find(|m| m.id == model_id)
        .unwrap_or_else(|| panic!("model {model_id} missing from registry.json"))
}

/// Locate the model directory. Prefers `VOICEPASTE_PARAKEET_MODEL_DIR`, then the
/// production app-data cache (read-only), so the benchmark never downloads.
fn model_dir(entry: &ModelEntry) -> Option<PathBuf> {
    let dir = std::env::var_os("VOICEPASTE_PARAKEET_MODEL_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            dirs::data_dir().map(|base| base.join("com.yolanda.voicepaste/models").join(&entry.id))
        })?;

    let complete = entry
        .model_files
        .values()
        .all(|filename| dir.join(filename).exists());
    complete.then_some(dir)
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

fn threads() -> u32 {
    std::env::var("VOICEPASTE_PARAKEET_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
}

fn transcript(recognizer: &sherpa_onnx::OfflineRecognizer, samples: &[f32]) -> String {
    let stream = recognizer.create_stream();
    stream.accept_waveform(16000, samples);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default()
}

/// Pick a bundled test wav: `en.wav` when present (v3 export), otherwise the
/// first wav by name (v2 ships `0.wav`). `VOICEPASTE_PARAKEET_WAV` overrides the
/// selection for long-audio / custom benchmarks.
fn test_wav(dir: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("VOICEPASTE_PARAKEET_WAV") {
        return PathBuf::from(path);
    }
    let wavs = dir.join("test_wavs");
    let en = wavs.join("en.wav");
    if en.exists() {
        return en;
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&wavs)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
                .collect()
        })
        .unwrap_or_default();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| wavs.join("en.wav"))
}

#[test]
fn parakeet_short_utterance_smoke() {
    let entry = registry_entry();
    let Some(dir) = model_dir(&entry) else {
        eprintln!("SKIP: {} not downloaded", entry.id);
        return;
    };
    let audio = test_wav(&dir);

    let load_start = Instant::now();
    let recognizer = build_nemo_transducer_recognizer(
        &dir,
        &entry,
        threads(),
        &serde_json::json!({ "provider": "cpu" }),
    )
    .expect("recognizer should load");
    let load_ms = load_start.elapsed().as_millis();

    let samples = read_wav(&audio).expect("decode test wav");
    let audio_secs = samples.len() as f64 / 16000.0;

    let start = Instant::now();
    let text = transcript(&recognizer, &samples);
    let elapsed = start.elapsed().as_secs_f64();

    eprintln!(
        "model={} threads={} load={load_ms}ms audio={audio_secs:.2}s decode={elapsed:.3}s rtf={:.3}",
        entry.id,
        threads(),
        elapsed / audio_secs
    );
    eprintln!("text: {text}");

    assert!(!text.is_empty(), "Parakeet should transcribe speech");
    assert!(
        text.split_whitespace().count() >= 3,
        "unexpected transcript: {text}"
    );
}

#[test]
fn parakeet_repeated_sessions_are_stable() {
    let entry = registry_entry();
    let Some(dir) = model_dir(&entry) else {
        eprintln!("SKIP: {} not downloaded", entry.id);
        return;
    };
    let samples = read_wav(&test_wav(&dir)).expect("decode test wav");
    let recognizer = build_nemo_transducer_recognizer(
        &dir,
        &entry,
        threads(),
        &serde_json::json!({ "provider": "cpu" }),
    )
    .expect("recognizer should load");

    let first = transcript(&recognizer, &samples);
    for round in 1..=2 {
        let text = transcript(&recognizer, &samples);
        eprintln!("round {round}: {text}");
        assert_eq!(text, first, "repeated decoding must be deterministic");
    }
}

#[test]
fn parakeet_silence_does_not_crash() {
    let entry = registry_entry();
    let Some(dir) = model_dir(&entry) else {
        eprintln!("SKIP: {} not downloaded", entry.id);
        return;
    };
    let recognizer = build_nemo_transducer_recognizer(
        &dir,
        &entry,
        threads(),
        &serde_json::json!({ "provider": "cpu" }),
    )
    .expect("recognizer should load");

    let silence = vec![0.0_f32; 16000];
    let text = transcript(&recognizer, &silence);
    eprintln!("silence transcript: '{text}'");
    assert!(
        text.len() < 200,
        "silence should not hallucinate a long text"
    );
}
