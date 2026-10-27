//! NVIDIA NeMo transducer models (Parakeet TDT family) via sherpa-onnx.
//!
//! sherpa-onnx exposes these exports through the offline transducer config plus
//! `model_type = "nemo_transducer"`; the model itself is a plain ONNX
//! encoder/decoder/joiner triple, so no NVIDIA toolkit or Python runtime is
//! needed at build or run time.

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::Path;

use crate::model::ModelEntry;

use super::json_string;

/// Model family selector understood by sherpa-onnx for NeMo exports.
pub(crate) const MODEL_TYPE: &str = "nemo_transducer";

/// Build a NeMo transducer `OfflineRecognizer` from a registry entry.
pub(crate) fn build_nemo_transducer_recognizer(
    model_dir: &Path,
    entry: &ModelEntry,
    num_threads: u32,
    model_config: &serde_json::Value,
) -> Result<OfflineRecognizer, String> {
    let config = nemo_transducer_config(model_dir, entry, num_threads, model_config)?;
    OfflineRecognizer::create(&config).ok_or_else(|| {
        format!(
            "创建 NeMo Transducer 识别器失败 (model: {})，请重新下载模型",
            entry.id
        )
    })
}

/// Resolve the sherpa-onnx config for a NeMo transducer entry.
///
/// Split out from [`build_nemo_transducer_recognizer`] so the file resolution
/// and model-family wiring are testable without loading a ~650 MB encoder.
pub(crate) fn nemo_transducer_config(
    model_dir: &Path,
    entry: &ModelEntry,
    num_threads: u32,
    model_config: &serde_json::Value,
) -> Result<OfflineRecognizerConfig, String> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.num_threads = num_threads as i32;
    config.model_config.debug = cfg!(debug_assertions);
    config.model_config.provider = json_string(model_config, "provider");

    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: Some(require_file(model_dir, entry, "encoder")?),
        decoder: Some(require_file(model_dir, entry, "decoder")?),
        joiner: Some(require_file(model_dir, entry, "joiner")?),
    };
    config.model_config.tokens = Some(require_file(model_dir, entry, "tokens")?);
    config.model_config.model_type = Some(MODEL_TYPE.to_string());

    Ok(config)
}

/// Resolve a required model file, naming the exact missing file in the error so
/// a partially extracted download is diagnosable from the recording error.
///
/// Zero-length files are rejected up front: sherpa-onnx's C API lets the
/// underlying ONNX Runtime exception escape, which aborts the process instead of
/// returning `None`. A size check turns the most common broken download into a
/// recoverable error message.
fn require_file(model_dir: &Path, entry: &ModelEntry, key: &str) -> Result<String, String> {
    let filename = entry
        .model_files
        .get(key)
        .map(String::as_str)
        .unwrap_or("<未配置>");
    let path = model_dir.join(filename);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(format!(
            "模型 {} 的 {} 文件缺失或为空（{}），请重新下载",
            entry.id, key, filename
        ));
    }
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("模型 {} 的 {} 路径无法解析", entry.id, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Capabilities, ModelCategory};
    use std::collections::HashMap;

    fn entry(files: &[(&str, &str)]) -> ModelEntry {
        ModelEntry {
            id: "test-nemo".to_string(),
            model_type: "offline".to_string(),
            category: ModelCategory::Asr,
            engine: "sherpa-onnx".to_string(),
            name: "Test NeMo".to_string(),
            description: String::new(),
            tags: Vec::new(),
            capabilities: Capabilities::default(),
            languages: vec!["en".to_string()],
            requires_config: None,
            architecture: Some("nemo_transducer".to_string()),
            download_url: None,
            file_size: None,
            mem_size: None,
            model_files: files
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
            default_config: None,
        }
    }

    fn full_entry() -> ModelEntry {
        entry(&[
            ("encoder", "encoder.int8.onnx"),
            ("decoder", "decoder.int8.onnx"),
            ("joiner", "joiner.int8.onnx"),
            ("tokens", "tokens.txt"),
        ])
    }

    #[test]
    fn missing_encoder_reports_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let err = nemo_transducer_config(dir.path(), &full_entry(), 2, &serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("encoder"), "unexpected error: {err}");
        assert!(err.contains("encoder.int8.onnx"), "unexpected error: {err}");
    }

    #[test]
    fn missing_tokens_reports_file_name() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"] {
            std::fs::write(dir.path().join(file), b"stub").unwrap();
        }
        let err = nemo_transducer_config(dir.path(), &full_entry(), 2, &serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("tokens"), "unexpected error: {err}");
    }

    #[test]
    fn config_uses_nemo_transducer_family_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
        ] {
            std::fs::write(dir.path().join(file), b"stub").unwrap();
        }

        let config = nemo_transducer_config(
            dir.path(),
            &full_entry(),
            6,
            &serde_json::json!({ "provider": "cpu" }),
        )
        .unwrap();

        assert_eq!(config.model_config.model_type.as_deref(), Some(MODEL_TYPE));
        assert_eq!(config.model_config.num_threads, 6);
        assert_eq!(config.model_config.provider.as_deref(), Some("cpu"));
        assert_eq!(
            config.model_config.transducer.encoder.as_deref(),
            Some(dir.path().join("encoder.int8.onnx").to_str().unwrap())
        );
        assert_eq!(
            config.model_config.transducer.decoder.as_deref(),
            Some(dir.path().join("decoder.int8.onnx").to_str().unwrap())
        );
        assert_eq!(
            config.model_config.transducer.joiner.as_deref(),
            Some(dir.path().join("joiner.int8.onnx").to_str().unwrap())
        );
        assert_eq!(
            config.model_config.tokens.as_deref(),
            Some(dir.path().join("tokens.txt").to_str().unwrap())
        );
    }

    #[test]
    fn empty_model_file_is_rejected_before_loading() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"] {
            std::fs::write(dir.path().join(file), b"stub").unwrap();
        }
        // Zero-byte encoder: must be rejected by validation, never handed to
        // ONNX Runtime (which would abort the process on the C++ exception).
        std::fs::write(dir.path().join("encoder.int8.onnx"), b"").unwrap();

        let err = match build_nemo_transducer_recognizer(
            dir.path(),
            &full_entry(),
            2,
            &serde_json::json!({}),
        ) {
            Ok(_) => panic!("empty model file must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("encoder"), "unexpected error: {err}");
        assert!(err.contains("为空"), "unexpected error: {err}");
    }
}
