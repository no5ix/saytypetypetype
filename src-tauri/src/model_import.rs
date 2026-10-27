//! Hugging Face repository imports for sherpa-ONNX ASR models.
//!
//! The intake is intentionally narrow: a repository must expose the same file
//! set the bundled NeMo transducer entry uses (encoder/decoder/joiner ONNX plus
//! `tokens.txt`). Repositories that only publish NeMo checkpoints, Transformers
//! or MLX weights are rejected with an explanation instead of being renamed and
//! registered as if they were loadable — no conversion and no second inference
//! runtime happens here.
//!
//! Downloads stream into a temporary directory under the app data dir, are
//! verified, and are then committed with a single `rename` into
//! `models/<internal-id>`. The registry entry is only written after that commit,
//! so a cancelled or failed import can never look like a usable model.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use crate::model::{self, Capabilities, ModelCategory, ModelEntry};

/// `download_url` scheme marking a registry entry as a local Hugging Face import.
pub(crate) const HF_URL_PREFIX: &str = "hf:";

/// Message returned when the user cancels an import. Shared with the UI so a
/// cancellation is not reported as an error.
pub(crate) const CANCELLED_MESSAGE: &str = "导入已取消";

const PROGRESS_EVENT: &str = "model:download:progress";
const MAX_REPO_PART_LEN: usize = 96;

static CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
static IMPORT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Public Hugging Face endpoints. Overridable so tests can point at a mock server.
#[derive(Debug, Clone)]
pub(crate) struct HfEndpoints {
    pub api_base: String,
    pub file_base: String,
}

impl Default for HfEndpoints {
    fn default() -> Self {
        Self {
            api_base: "https://huggingface.co".to_string(),
            file_base: "https://huggingface.co".to_string(),
        }
    }
}

/// A validated `owner/model` repository ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HfRepo {
    pub owner: String,
    pub name: String,
}

impl HfRepo {
    pub(crate) fn id(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Parse and validate a Hugging Face repository ID (`owner/model-name`).
pub(crate) fn parse_repo_id(input: &str) -> Result<HfRepo, String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(
            "请输入 Hugging Face 仓库 ID，格式为 owner/model，例如 nvidia/parakeet-tdt-0.6b-v2"
                .to_string(),
        );
    }
    let mut segments = trimmed.split('/');
    let (owner, name) = match (segments.next(), segments.next(), segments.next()) {
        (Some(owner), Some(name), None) => (owner, name),
        _ => {
            return Err(
                "仓库 ID 格式应为 owner/model（例如 nvidia/parakeet-tdt-0.6b-v2）".to_string(),
            )
        }
    };
    for (label, part) in [("owner", owner), ("model", name)] {
        if !is_valid_repo_part(part) {
            return Err(format!(
                "仓库 ID 的 {label} 部分无效：{part}（只允许字母、数字、. _ -）"
            ));
        }
    }
    Ok(HfRepo {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

fn is_valid_repo_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= MAX_REPO_PART_LEN
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Recover the repository ID from a registry `download_url` created by an import.
pub(crate) fn repo_from_hf_url(url: &str) -> Option<String> {
    let repo = url.strip_prefix(HF_URL_PREFIX)?;
    parse_repo_id(repo).ok().map(|repo| repo.id())
}

/// Stable, filesystem-safe internal model ID for a repository.
///
/// The sanitized `owner-name` keeps the ID readable in the UI and on disk; the
/// short digest keeps distinct repositories that sanitize to the same slug
/// apart, and keeps the ID stable across re-imports.
pub(crate) fn internal_model_id(repo: &HfRepo) -> String {
    let owner = {
        let slug = slugify(&repo.owner);
        if slug.is_empty() {
            "repo".to_string()
        } else {
            slug
        }
    };
    let name = {
        let slug = slugify(&repo.name);
        if slug.is_empty() {
            "model".to_string()
        } else {
            slug
        }
    };
    let digest = fnv1a(&repo.id().to_ascii_lowercase()) as u32;
    format!("hf-{owner}-{name}-{digest:08x}")
}

fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    let truncated: String = out.chars().take(32).collect();
    truncated.trim_matches('-').to_string()
}

fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Human-readable model name derived from the repository name, e.g.
/// `parakeet-tdt-0.6b-v2` → `Parakeet TDT 0.6B v2`.
pub(crate) fn display_name(repo: &HfRepo) -> String {
    let words: Vec<String> = repo
        .name
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let is_version = lower.len() > 1
                && lower.starts_with('v')
                && lower[1..].chars().all(|c| c.is_ascii_digit());
            if is_version {
                return lower;
            }
            if let Some(acronym) = acronym(&lower) {
                return acronym.to_string();
            }
            if is_size_token(&lower) {
                let mut sized = lower[..lower.len() - 1].to_string();
                sized.push('B');
                return sized;
            }
            capitalize(word)
        })
        .collect();
    if words.is_empty() {
        repo.name.clone()
    } else {
        words.join(" ")
    }
}

/// Short acronyms that must stay upper-cased in display names. Repository names
/// are lowercase, so a plain capitalize() would render `tdt` as `Tdt`.
fn acronym(token: &str) -> Option<&'static str> {
    match token {
        "asr" => Some("ASR"),
        "ctc" => Some("CTC"),
        "hf" => Some("HF"),
        "mlx" => Some("MLX"),
        "nemo" => Some("NeMo"),
        "onnx" => Some("ONNX"),
        "rnnt" => Some("RNNT"),
        "stt" => Some("STT"),
        "tdt" => Some("TDT"),
        "vad" => Some("VAD"),
        _ => None,
    }
}

fn is_size_token(token: &str) -> bool {
    let Some(stem) = token.strip_suffix('b') else {
        return false;
    };
    let mut parts = stem.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some() {
        return false;
    }
    !integer.is_empty()
        && integer.chars().all(|c| c.is_ascii_digit())
        && fraction.is_none_or(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// One file in a repository listing.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct HfSibling {
    pub(crate) rfilename: String,
    #[serde(default)]
    pub(crate) size: Option<u64>,
}

impl HfSibling {
    #[cfg(test)]
    pub(crate) fn new(name: &str, size: Option<u64>) -> Self {
        Self {
            rfilename: name.to_string(),
            size,
        }
    }
}

/// The repository paths an import will fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportFiles {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
    /// External tensor-data files that belong to the chosen ONNX models.
    pub extras: Vec<String>,
    pub total_bytes: Option<u64>,
}

impl ImportFiles {
    /// Every repository path to download, required files first.
    fn downloads(&self) -> Vec<&str> {
        let mut paths = vec![
            self.encoder.as_str(),
            self.decoder.as_str(),
            self.joiner.as_str(),
            self.tokens.as_str(),
        ];
        paths.extend(self.extras.iter().map(String::as_str));
        paths
    }

    /// Registry `model_files` map: key → file name inside the model directory.
    fn destinations(&self) -> Result<HashMap<String, String>, String> {
        let mut files = HashMap::new();
        for (key, path) in [
            ("encoder", &self.encoder),
            ("decoder", &self.decoder),
            ("joiner", &self.joiner),
            ("tokens", &self.tokens),
        ] {
            files.insert(key.to_string(), basename(path)?.to_string());
        }
        Ok(files)
    }
}

/// Resolve the sherpa-ONNX file set from a repository listing.
///
/// The NeMo transducer loader needs separate encoder/decoder/joiner ONNX models
/// plus a `tokens.txt`. Combined `decoder_joint` exports (onnxruntime style) and
/// preprocessor side-models are not compatible, so they are not accepted as
/// substitutes.
pub(crate) fn resolve_import_files(siblings: &[HfSibling]) -> Result<ImportFiles, String> {
    let encoder = pick(siblings, |name| {
        name.ends_with(".onnx") && name.contains("encoder")
    });
    let decoder = pick(siblings, |name| {
        name.ends_with(".onnx") && name.contains("decoder") && !name.contains("joint")
    });
    let joiner = pick(siblings, |name| {
        name.ends_with(".onnx") && name.contains("joiner")
    });
    let tokens = pick_tokens(siblings);

    let mut missing = Vec::new();
    if encoder.is_none() {
        missing.push("encoder");
    }
    if decoder.is_none() {
        missing.push("decoder");
    }
    if joiner.is_none() {
        missing.push("joiner");
    }
    if tokens.is_none() {
        missing.push("tokens");
    }
    if !missing.is_empty() {
        return Err(unsupported_repository_error(siblings, &missing));
    }

    let encoder = encoder.expect("checked above");
    let decoder = decoder.expect("checked above");
    let joiner = joiner.expect("checked above");
    let tokens = tokens.expect("checked above");
    // Reject path traversal before anything is written.
    for file in [&encoder, &decoder, &joiner, &tokens] {
        basename(&file.rfilename)?;
    }

    let mut extras: Vec<String> = Vec::new();
    for file in [&encoder, &decoder, &joiner] {
        for suffix in [".data", "_data"] {
            let candidate = format!("{}{suffix}", file.rfilename);
            if let Some(found) = siblings.iter().find(|other| other.rfilename == candidate) {
                basename(&found.rfilename)?;
                extras.push(found.rfilename.clone());
            }
        }
    }

    let mut total = Some(0u64);
    for file in [&encoder, &decoder, &joiner, &tokens] {
        match file.size {
            Some(size) => total = total.map(|sum| sum + size),
            None => total = None,
        }
    }
    for extra in &extras {
        match siblings
            .iter()
            .find(|file| &file.rfilename == extra)
            .and_then(|file| file.size)
        {
            Some(size) => total = total.map(|sum| sum + size),
            None => total = None,
        }
    }

    Ok(ImportFiles {
        encoder: encoder.rfilename,
        decoder: decoder.rfilename,
        joiner: joiner.rfilename,
        tokens: tokens.rfilename,
        extras,
        total_bytes: total.filter(|total| *total > 0),
    })
}

/// Pick a matching file, preferring int8 exports and otherwise the first name in
/// sorted order so a repository always resolves to the same files.
fn pick(siblings: &[HfSibling], matches: impl Fn(&str) -> bool) -> Option<HfSibling> {
    let mut candidates: Vec<&HfSibling> = siblings
        .iter()
        .filter(|file| matches(&file.rfilename.to_ascii_lowercase()))
        .collect();
    candidates.sort_by(|a, b| a.rfilename.cmp(&b.rfilename));
    candidates
        .iter()
        .copied()
        .find(|file| file.rfilename.to_ascii_lowercase().contains("int8"))
        .or_else(|| candidates.first().copied())
        .cloned()
}

fn pick_tokens(siblings: &[HfSibling]) -> Option<HfSibling> {
    siblings
        .iter()
        .find(|file| file.rfilename.eq_ignore_ascii_case("tokens.txt"))
        .or_else(|| {
            siblings
                .iter()
                .find(|file| file.rfilename.to_ascii_lowercase().ends_with("tokens.txt"))
        })
        .cloned()
}

fn basename(path: &str) -> Result<&str, String> {
    let name = path.rsplit('/').next().unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." || name.contains('\\') {
        return Err(format!("仓库文件路径无效：{path}"));
    }
    Ok(name)
}

fn unsupported_repository_error(siblings: &[HfSibling], missing: &[&str]) -> String {
    let labels: Vec<&str> = missing
        .iter()
        .map(|key| match *key {
            "encoder" => "encoder .onnx",
            "decoder" => "decoder .onnx",
            "joiner" => "joiner .onnx",
            _ => "tokens.txt",
        })
        .collect();
    let mut message = format!(
        "无法导入：该仓库缺少 sherpa-ONNX 需要的 {} 文件。VoicePaste 只能直接加载 \
         sherpa-ONNX 格式的 NeMo transducer 模型（encoder/decoder/joiner .onnx + tokens.txt）。",
        labels.join("、")
    );
    let formats = formats_found(siblings);
    if !formats.is_empty() {
        message.push_str(&format!(
            "仓库中的文件格式为：{}。原始 NeMo/MLX 权重需要先转换为 sherpa-ONNX 格式。",
            formats.join("、")
        ));
    }
    message
}

fn formats_found(siblings: &[HfSibling]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for file in siblings {
        let name = file.rfilename.to_ascii_lowercase();
        let label = if name.ends_with(".nemo") {
            "NeMo 检查点（.nemo）"
        } else if name.ends_with(".onnx") {
            "其他 ONNX 布局"
        } else if name.ends_with(".safetensors") {
            "Transformers 权重（.safetensors）"
        } else if name.ends_with(".gguf") {
            "GGUF 权重（.gguf）"
        } else if name.contains("mlx") {
            "MLX 权重"
        } else if name.ends_with(".npz")
            || name.ends_with(".pt")
            || name.ends_with(".pth")
            || name.ends_with(".bin")
        {
            "PyTorch 权重"
        } else {
            continue;
        };
        if !found.iter().any(|existing| existing == label) {
            found.push(label.to_string());
        }
    }
    found
}

/// Build the registry entry for an imported repository.
pub(crate) fn entry_for(repo: &HfRepo, files: &ImportFiles) -> Result<ModelEntry, String> {
    Ok(ModelEntry {
        id: internal_model_id(repo),
        model_type: "offline".to_string(),
        category: ModelCategory::Asr,
        engine: "sherpa-onnx".to_string(),
        name: display_name(repo),
        description: format!("从 Hugging Face 导入（{}）", repo.id()),
        tags: vec!["Hugging Face 导入".to_string(), "本地模型".to_string()],
        capabilities: Capabilities::default(),
        languages: Vec::new(),
        requires_config: None,
        architecture: Some("nemo_transducer".to_string()),
        download_url: Some(format!("{HF_URL_PREFIX}{}", repo.id())),
        file_size: files
            .total_bytes
            .map(|bytes| bytes.div_ceil(1_000_000).max(1)),
        mem_size: None,
        model_files: files.destinations()?,
        default_config: None,
    })
}

/// Result of an import request.
#[derive(Debug)]
pub(crate) struct ImportOutcome {
    pub entry: ModelEntry,
    pub already_imported: bool,
}

/// Guards against concurrent imports and clears a stale cancel request.
struct ActiveImport;

impl ActiveImport {
    fn acquire() -> Result<Self, String> {
        if IMPORT_ACTIVE.swap(true, Ordering::SeqCst) {
            return Err("已有模型正在导入，请等待当前导入结束".to_string());
        }
        CANCEL_REQUESTED.store(false, Ordering::SeqCst);
        Ok(Self)
    }
}

impl Drop for ActiveImport {
    fn drop(&mut self) {
        IMPORT_ACTIVE.store(false, Ordering::SeqCst);
        CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    }
}

/// Ask the running import (if any) to stop after the current chunk.
pub(crate) fn request_cancel() {
    CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

fn check_cancel() -> Result<(), String> {
    if CANCEL_REQUESTED.load(Ordering::SeqCst) {
        Err(CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

fn emit_progress(
    app: Option<&AppHandle>,
    model_id: &str,
    status: &str,
    progress: u32,
    message: Option<&str>,
) {
    let Some(app) = app else {
        return;
    };
    let mut payload = serde_json::json!({
        "model_id": model_id,
        "status": status,
        "progress": progress,
    });
    if let Some(message) = message {
        payload["message"] = serde_json::Value::String(message.to_string());
    }
    let _ = app.emit(PROGRESS_EVENT, payload);
}

/// Download, prepare and register a repository as a local ASR model.
pub(crate) async fn import_repo(
    app: Option<&AppHandle>,
    data_dir: &Path,
    resource_dir: &Path,
    repo_id: &str,
    endpoints: &HfEndpoints,
) -> Result<ImportOutcome, String> {
    let repo = parse_repo_id(repo_id)?;
    let _active = ActiveImport::acquire()?;
    let model_id = internal_model_id(&repo);
    let models_root = model::models_dir(data_dir);

    // Idempotent re-import: a completed import stays untouched, and a registry
    // entry whose files were removed is repaired by re-downloading below.
    let registry = model::load_registry(data_dir, resource_dir);
    if let Some(entry) = registry.models.iter().find(|model| model.id == model_id) {
        if model::is_model_downloaded_at(&models_root, entry) {
            emit_progress(app, &model_id, "completed", 100, None);
            return Ok(ImportOutcome {
                entry: entry.clone(),
                already_imported: true,
            });
        }
    }

    emit_progress(app, &model_id, "downloading", 0, Some("读取仓库文件列表"));
    let siblings = fetch_siblings(&repo, endpoints).await?;
    check_cancel()?;
    let files = resolve_import_files(&siblings)?;
    let entry = entry_for(&repo, &files)?;

    let staging = staging_dir(data_dir, &model_id);
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| format!("清理临时目录失败: {e}"))?;
    }
    fs::create_dir_all(&staging).map_err(|e| format!("创建临时目录失败: {e}"))?;

    let result = stage_files(app, endpoints, &repo, &model_id, &files, &staging)
        .await
        .and_then(|()| finalize_import(data_dir, resource_dir, &entry, &staging));
    if result.is_err() {
        // Never leave a partial download behind; the registry was not touched.
        let _ = fs::remove_dir_all(&staging);
    }
    result?;

    emit_progress(app, &model_id, "completed", 100, None);
    log_app!(
        info,
        "Imported Hugging Face model {} from {}",
        model_id,
        repo.id()
    );
    Ok(ImportOutcome {
        entry,
        already_imported: false,
    })
}

fn staging_dir(data_dir: &Path, model_id: &str) -> PathBuf {
    data_dir.join("tmp").join(format!("import-{model_id}"))
}

async fn fetch_siblings(repo: &HfRepo, endpoints: &HfEndpoints) -> Result<Vec<HfSibling>, String> {
    #[derive(Deserialize)]
    struct HfModelInfo {
        #[serde(default)]
        siblings: Vec<HfSibling>,
    }

    let url = format!(
        "{}/api/models/{}?blobs=true",
        endpoints.api_base.trim_end_matches('/'),
        encode_path(&repo.id()),
    );
    let response = http_client()?
        .get(&url)
        .header("accept", "application/json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("连接 Hugging Face 失败: {e}"))?;

    match response.status().as_u16() {
        200 => {}
        401 | 403 => {
            return Err(format!(
                "无法访问 {}：该仓库需要登录或授权，VoicePaste 只导入公开仓库",
                repo.id()
            ))
        }
        404 => return Err(format!("未找到仓库 {}，请检查仓库 ID 是否正确", repo.id())),
        status => return Err(format!("获取仓库信息失败: HTTP {status}")),
    }

    let info: HfModelInfo = response
        .json()
        .await
        .map_err(|e| format!("解析仓库信息失败: {e}"))?;
    if info.siblings.is_empty() {
        return Err(format!("仓库 {} 中没有文件", repo.id()));
    }
    Ok(info.siblings)
}

async fn stage_files(
    app: Option<&AppHandle>,
    endpoints: &HfEndpoints,
    repo: &HfRepo,
    model_id: &str,
    files: &ImportFiles,
    staging: &Path,
) -> Result<(), String> {
    let client = http_client()?;
    let total = files.total_bytes;
    let mut downloaded: u64 = 0;
    let mut last_percent = u32::MAX;

    for path in files.downloads() {
        check_cancel()?;
        let name = basename(path)?;
        let label = format!("下载 {name}");
        let url = format!(
            "{}/{}/resolve/main/{}",
            endpoints.file_base.trim_end_matches('/'),
            encode_path(&repo.id()),
            encode_path(path),
        );
        emit_progress(
            app,
            model_id,
            "downloading",
            progress_percent(downloaded, total),
            Some(&label),
        );
        let mut on_bytes = |written: u64| {
            let overall = downloaded + written;
            let percent = progress_percent(overall, total);
            if percent != last_percent {
                last_percent = percent;
                emit_progress(app, model_id, "downloading", percent, Some(&label));
            }
        };
        let written = download_file(&client, &url, &staging.join(name), &mut on_bytes).await?;
        downloaded += written;
    }
    Ok(())
}

fn progress_percent(downloaded: u64, total: Option<u64>) -> u32 {
    match total {
        Some(total) if total > 0 => ((downloaded.min(total)) as f64 / total as f64 * 100.0) as u32,
        _ => 0,
    }
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    on_bytes: &mut impl FnMut(u64),
) -> Result<u64, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "下载失败: 该仓库需要登录或授权（HTTP 401/403）".to_string(),
            404 => "下载失败: 仓库文件不存在（HTTP 404）".to_string(),
            status => format!("下载失败: HTTP {status}"),
        });
    }

    let mut file = fs::File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut stream = response.bytes_stream();
    let mut written: u64 = 0;
    loop {
        check_cancel()?;
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败: {e}"))?;
        written += chunk.len() as u64;
        on_bytes(written);
    }
    file.flush().map_err(|e| format!("写入文件失败: {e}"))?;
    if written == 0 {
        return Err("下载失败: 服务器返回了空文件".to_string());
    }
    Ok(written)
}

/// Commit a fully downloaded staging directory and register the model.
///
/// The `rename` is the atomic step: either the complete directory becomes
/// `models/<id>` or nothing does. The registry entry is written afterwards, and
/// rolled back if that write fails.
pub(crate) fn finalize_import(
    data_dir: &Path,
    resource_dir: &Path,
    entry: &ModelEntry,
    staging: &Path,
) -> Result<(), String> {
    let models_root = model::models_dir(data_dir);
    let final_dir = models_root.join(&entry.id);

    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|e| format!("清理旧模型目录失败: {e}"))?;
    }
    fs::create_dir_all(&models_root).map_err(|e| format!("创建模型目录失败: {e}"))?;
    fs::rename(staging, &final_dir).map_err(|e| format!("保存模型目录失败: {e}"))?;

    if !model::is_model_downloaded_at(&models_root, entry) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(format!("导入失败：模型 {} 的文件不完整", entry.id));
    }

    if let Err(error) = model::upsert_registry_entry(data_dir, resource_dir, entry.clone()) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(error);
    }
    Ok(())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("VoicePaste/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))
}

/// Percent-encode a repository path while keeping `/` as a separator.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelRegistry;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const REPO: &str = "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
    const SHERPA_FILES: [&str; 4] = [
        "encoder.int8.onnx",
        "decoder.int8.onnx",
        "joiner.int8.onnx",
        "tokens.txt",
    ];

    /// The import guard is process-wide (the app runs one import at a time), so
    /// tests must not start two imports concurrently.
    static IMPORT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn serial_import() -> tokio::sync::MutexGuard<'static, ()> {
        IMPORT_TEST_LOCK.lock().await
    }

    fn siblings(names: &[&str]) -> Vec<HfSibling> {
        names
            .iter()
            .map(|name| HfSibling::new(name, Some(10)))
            .collect()
    }

    fn sherpa_files() -> ImportFiles {
        resolve_import_files(&siblings(&SHERPA_FILES)).unwrap()
    }

    fn fixture_registry() -> serde_json::Value {
        json!({
            "version": 1,
            "updated_at": "2026-01-01",
            "models": [{
                "id": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
                "type": "offline",
                "category": "asr",
                "engine": "sherpa-onnx",
                "name": "Parakeet TDT 0.6B v3",
                "description": "bundled",
                "tags": [],
                "capabilities": { "streaming": false },
                "architecture": "nemo_transducer",
                "download_url": "https://example.com/v3.tar.bz2",
                "file_size": 487,
                "model_files": {
                    "encoder": "encoder.int8.onnx",
                    "decoder": "decoder.int8.onnx",
                    "joiner": "joiner.int8.onnx",
                    "tokens": "tokens.txt"
                }
            }]
        })
    }

    fn fixture_dirs() -> (TempDir, TempDir) {
        let data = tempfile::tempdir().unwrap();
        let resources = tempfile::tempdir().unwrap();
        fs::write(
            resources.path().join("registry.json"),
            fixture_registry().to_string(),
        )
        .unwrap();
        (data, resources)
    }

    fn stage_model(dir: &Path, entry: &ModelEntry, contents: &[u8]) {
        fs::create_dir_all(dir).unwrap();
        for name in entry.model_files.values() {
            fs::write(dir.join(name), contents).unwrap();
        }
    }

    fn registry_of(data: &Path, resources: &Path) -> ModelRegistry {
        model::load_registry(data, resources)
    }

    fn endpoints_for(server: &MockServer) -> HfEndpoints {
        HfEndpoints {
            api_base: server.uri(),
            file_base: server.uri(),
        }
    }

    async fn mount_repo_listing(server: &MockServer, repo: &str, names: &[&str]) {
        let siblings: Vec<serde_json::Value> = names
            .iter()
            .map(|name| json!({ "rfilename": name, "size": 4 }))
            .collect();
        Mock::given(method("GET"))
            .and(path(format!("/api/models/{repo}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "siblings": siblings })))
            .mount(server)
            .await;
    }

    async fn mount_file(
        server: &MockServer,
        repo: &str,
        name: &str,
        body: &[u8],
        delay: Option<Duration>,
    ) {
        let mut template = ResponseTemplate::new(200).set_body_bytes(body.to_vec());
        if let Some(delay) = delay {
            template = template.set_delay(delay);
        }
        Mock::given(method("GET"))
            .and(path(format!("/{repo}/resolve/main/{name}")))
            .respond_with(template)
            .mount(server)
            .await;
    }

    #[test]
    fn parses_and_validates_repository_ids() {
        let repo = parse_repo_id("  nvidia/parakeet-tdt-0.6b-v2/  ").unwrap();
        assert_eq!(repo.id(), "nvidia/parakeet-tdt-0.6b-v2");

        for bad in [
            "",
            "   ",
            "parakeet",
            "a/b/c",
            "/model",
            "owner/",
            "owner/..",
            "../x/y",
            "own er/model",
            "owner/mo del",
        ] {
            assert!(parse_repo_id(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn builds_filesystem_safe_internal_ids() {
        let repo = parse_repo_id("nvidia/parakeet-tdt-0.6b-v2").unwrap();
        let id = internal_model_id(&repo);

        assert!(id.starts_with("hf-nvidia-parakeet-tdt-0-6b-v2-"));
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_eq!(
            Path::new(&id).components().count(),
            1,
            "id must be one path segment"
        );
        // Never reuse the bundled v3 identity, and stay stable across re-imports.
        assert_ne!(id, "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8");
        assert_eq!(
            id,
            internal_model_id(&parse_repo_id("nvidia/parakeet-tdt-0.6b-v2").unwrap())
        );
        assert_ne!(
            id,
            internal_model_id(&parse_repo_id("nvidia/parakeet-tdt-0.6b-v3").unwrap())
        );
        assert_ne!(
            internal_model_id(&parse_repo_id("org/model").unwrap()),
            internal_model_id(&parse_repo_id("org/model_v1").unwrap()),
            "distinct repositories must not share a directory"
        );
    }

    #[test]
    fn builds_a_readable_display_name() {
        assert_eq!(
            display_name(&parse_repo_id("nvidia/parakeet-tdt-0.6b-v2").unwrap()),
            "Parakeet TDT 0.6B v2"
        );
        assert_eq!(
            display_name(&parse_repo_id(REPO).unwrap()),
            "Sherpa ONNX NeMo Parakeet TDT 0.6B v3 Int8"
        );
    }

    #[test]
    fn resolves_the_sherpa_onnx_file_set() {
        let files = resolve_import_files(&siblings(&[
            "README.md",
            "encoder.onnx",
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
            "test_wavs/en.wav",
        ]))
        .unwrap();

        assert_eq!(
            files.encoder, "encoder.int8.onnx",
            "int8 exports are preferred"
        );
        assert_eq!(files.decoder, "decoder.int8.onnx");
        assert_eq!(files.joiner, "joiner.int8.onnx");
        assert_eq!(files.tokens, "tokens.txt");
        assert_eq!(files.total_bytes, Some(40));
        assert!(files.extras.is_empty());

        let repo = parse_repo_id("org/model").unwrap();
        let entry = entry_for(&repo, &files).unwrap();
        assert_eq!(entry.engine, "sherpa-onnx");
        assert_eq!(entry.architecture.as_deref(), Some("nemo_transducer"));
        assert_eq!(entry.download_url.as_deref(), Some("hf:org/model"));
        assert_eq!(entry.model_files.len(), 4);
        assert_eq!(entry.model_files["encoder"], "encoder.int8.onnx");
    }

    #[test]
    fn resolves_nested_tokens_and_external_weights() {
        let files = resolve_import_files(&siblings(&[
            "onnx/encoder.onnx",
            "onnx/encoder.onnx.data",
            "onnx/decoder.onnx",
            "onnx/joiner.onnx",
            "onnx/tokens.txt",
        ]))
        .unwrap();

        assert_eq!(files.tokens, "onnx/tokens.txt");
        assert_eq!(files.extras, vec!["onnx/encoder.onnx.data".to_string()]);
        assert_eq!(files.total_bytes, Some(50));

        let repo = parse_repo_id("org/model").unwrap();
        let entry = entry_for(&repo, &files).unwrap();
        assert_eq!(entry.model_files["tokens"], "tokens.txt", "files land flat");
    }

    #[test]
    fn rejects_repositories_the_runtime_cannot_load() {
        // Combined decoder_joint export (onnxruntime style) is not sherpa-onnx compatible.
        let error = resolve_import_files(&siblings(&[
            "encoder-model.onnx",
            "decoder_joint-model.int8.onnx",
            "vocab.txt",
        ]))
        .unwrap_err();
        assert!(error.contains("decoder"), "{error}");
        assert!(error.contains("joiner"), "{error}");

        // The real nvidia/parakeet-tdt-0.6b-v2 repository only ships a NeMo checkpoint.
        let error = resolve_import_files(&siblings(&["README.md", "parakeet-tdt-0.6b-v2.nemo"]))
            .unwrap_err();
        assert!(error.contains("encoder"), "{error}");
        assert!(error.contains("NeMo"), "{error}");

        // MLX / Transformers weights only.
        let error =
            resolve_import_files(&siblings(&["config.json", "model.safetensors"])).unwrap_err();
        assert!(error.contains("safetensors"), "{error}");

        // Missing tokens only.
        let error = resolve_import_files(&siblings(&[
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
        ]))
        .unwrap_err();
        assert!(error.contains("tokens.txt"), "{error}");
    }

    #[test]
    fn imports_are_idempotent_and_preserve_the_existing_registry() {
        let (data, resources) = fixture_dirs();
        let repo = parse_repo_id(REPO).unwrap();
        let files = sherpa_files();
        let entry = entry_for(&repo, &files).unwrap();

        let staging = staging_dir(data.path(), &entry.id);
        stage_model(&staging, &entry, b"onnx");
        finalize_import(data.path(), resources.path(), &entry, &staging).unwrap();

        assert!(
            !staging.exists(),
            "staging directory is consumed by the rename"
        );
        let model_dir = model::model_path(data.path(), &entry.id);
        assert!(model_dir.exists());
        assert_eq!(fs::read_dir(&model_dir).unwrap().count(), 4);

        let registry = registry_of(data.path(), resources.path());
        assert_eq!(
            registry
                .models
                .iter()
                .filter(|model| model.id == entry.id)
                .count(),
            1
        );
        assert!(registry
            .models
            .iter()
            .any(|model| model.id == "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"));
        assert!(model::get_downloaded_models(data.path(), &registry).contains(&entry.id));

        // A second import of the same directory must not duplicate the entry.
        let staging = staging_dir(data.path(), &entry.id);
        stage_model(&staging, &entry, b"onnx");
        finalize_import(data.path(), resources.path(), &entry, &staging).unwrap();
        let registry = registry_of(data.path(), resources.path());
        assert_eq!(
            registry
                .models
                .iter()
                .filter(|model| model.id == entry.id)
                .count(),
            1
        );
    }

    #[test]
    fn incomplete_or_empty_downloads_never_reach_the_registry() {
        let (data, resources) = fixture_dirs();
        let repo = parse_repo_id(REPO).unwrap();
        let entry = entry_for(&repo, &sherpa_files()).unwrap();

        // Missing required file.
        let staging = staging_dir(data.path(), &entry.id);
        stage_model(&staging, &entry, b"onnx");
        fs::remove_file(staging.join("joiner.int8.onnx")).unwrap();
        let error = finalize_import(data.path(), resources.path(), &entry, &staging).unwrap_err();
        assert!(error.contains("不完整"), "{error}");
        assert!(!model::model_path(data.path(), &entry.id).exists());
        assert!(!registry_of(data.path(), resources.path())
            .models
            .iter()
            .any(|model| model.id == entry.id));

        // Zero-length files are treated as missing.
        let staging = staging_dir(data.path(), &entry.id);
        stage_model(&staging, &entry, b"");
        let error = finalize_import(data.path(), resources.path(), &entry, &staging).unwrap_err();
        assert!(error.contains("不完整"), "{error}");
        assert!(!model::model_path(data.path(), &entry.id).exists());
    }

    #[test]
    fn recovers_the_repository_from_the_registry_url() {
        assert_eq!(
            repo_from_hf_url("hf:org/model"),
            Some("org/model".to_string())
        );
        assert_eq!(repo_from_hf_url("https://example.com/model.tar.bz2"), None);
        assert_eq!(repo_from_hf_url("hf:not-a-repo"), None);
    }

    #[tokio::test]
    async fn already_imported_models_skip_the_network() {
        let _serial = serial_import().await;
        let (data, resources) = fixture_dirs();
        let repo = parse_repo_id(REPO).unwrap();
        let entry = entry_for(&repo, &sherpa_files()).unwrap();
        let staging = staging_dir(data.path(), &entry.id);
        stage_model(&staging, &entry, b"onnx");
        finalize_import(data.path(), resources.path(), &entry, &staging).unwrap();

        let endpoints = HfEndpoints {
            api_base: "http://127.0.0.1:1".to_string(),
            file_base: "http://127.0.0.1:1".to_string(),
        };
        let outcome = import_repo(None, data.path(), resources.path(), REPO, &endpoints)
            .await
            .unwrap();

        assert!(outcome.already_imported);
        assert_eq!(outcome.entry.id, entry.id);
    }

    #[tokio::test]
    async fn downloads_files_over_http_and_registers_the_model() {
        let _serial = serial_import().await;
        let server = MockServer::start().await;
        mount_repo_listing(&server, "org/model", &SHERPA_FILES).await;
        for name in SHERPA_FILES {
            mount_file(&server, "org/model", name, b"onnx", None).await;
        }
        let (data, resources) = fixture_dirs();

        let outcome = import_repo(
            None,
            data.path(),
            resources.path(),
            "org/model",
            &endpoints_for(&server),
        )
        .await
        .unwrap();

        assert!(!outcome.already_imported);
        assert_eq!(
            outcome.entry.id,
            internal_model_id(&parse_repo_id("org/model").unwrap())
        );
        let model_dir = model::model_path(data.path(), &outcome.entry.id);
        for name in SHERPA_FILES {
            assert_eq!(fs::read(model_dir.join(name)).unwrap(), b"onnx", "{name}");
        }
        assert!(registry_of(data.path(), resources.path())
            .models
            .iter()
            .any(|model| model.id == outcome.entry.id));
        assert!(!outcome.entry.model_files.is_empty());
        // Nothing is left in the staging area after a successful commit.
        assert!(!staging_dir(data.path(), &outcome.entry.id).exists());
    }

    #[tokio::test]
    async fn download_failures_leave_no_partial_model() {
        let _serial = serial_import().await;
        let server = MockServer::start().await;
        mount_repo_listing(&server, "org/model", &SHERPA_FILES).await;
        mount_file(&server, "org/model", "encoder.int8.onnx", b"onnx", None).await;
        // joiner/decoder/tokens are not mounted, so wiremock answers 404.
        let (data, resources) = fixture_dirs();
        let repo = parse_repo_id("org/model").unwrap();

        let error = import_repo(
            None,
            data.path(),
            resources.path(),
            "org/model",
            &endpoints_for(&server),
        )
        .await
        .unwrap_err();

        assert!(error.contains("404"), "{error}");
        let id = internal_model_id(&repo);
        assert!(!model::model_path(data.path(), &id).exists());
        assert!(!data
            .path()
            .join("tmp")
            .join(format!("import-{id}"))
            .exists());
        assert!(!registry_of(data.path(), resources.path())
            .models
            .iter()
            .any(|model| model.id == id));
    }

    #[tokio::test]
    async fn cancellation_removes_the_partial_download() {
        let _serial = serial_import().await;
        let server = MockServer::start().await;
        mount_repo_listing(&server, "org/model", &SHERPA_FILES).await;
        for name in SHERPA_FILES {
            mount_file(
                &server,
                "org/model",
                name,
                &vec![7u8; 4096],
                Some(Duration::from_millis(400)),
            )
            .await;
        }
        let (data, resources) = fixture_dirs();
        let data_path = data.path().to_path_buf();
        let resources_path = resources.path().to_path_buf();
        let endpoints = endpoints_for(&server);

        let task = tokio::spawn(async move {
            import_repo(None, &data_path, &resources_path, "org/model", &endpoints).await
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        request_cancel();

        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error, CANCELLED_MESSAGE);

        let id = internal_model_id(&parse_repo_id("org/model").unwrap());
        assert!(!model::model_path(data.path(), &id).exists());
        assert!(!data
            .path()
            .join("tmp")
            .join(format!("import-{id}"))
            .exists());
        assert!(!registry_of(data.path(), resources.path())
            .models
            .iter()
            .any(|model| model.id == id));
    }

    #[tokio::test]
    async fn rejects_unknown_repositories_with_a_useful_message() {
        let _serial = serial_import().await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models/org/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let (data, resources) = fixture_dirs();

        let error = import_repo(
            None,
            data.path(),
            resources.path(),
            "org/missing",
            &endpoints_for(&server),
        )
        .await
        .unwrap_err();

        assert!(error.contains("未找到仓库"), "{error}");
        assert!(!registry_of(data.path(), resources.path())
            .models
            .iter()
            .any(|model| model.id.starts_with("hf-org-missing")));
    }
}
