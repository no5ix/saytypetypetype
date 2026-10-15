//! ElevenLabs realtime STT. Protocol details stay behind the shared ASR traits.
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{sleep_until, timeout, Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::Request, Message},
    WebSocketStream,
};

use super::{AsrEngine, AsrEvent, AsrSession};
use crate::config::ElevenLabsRealtimeConfig;

const ENDPOINT: &str = "wss://api.elevenlabs.io/v1/speech-to-text/realtime";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FINAL_WINDOW: Duration = Duration::from_secs(10);
// The protocol has no commit ID. Briefly drain adjacent VAD events instead of
// treating the first post-stop commit as an end-of-stream acknowledgement.
const FINAL_SETTLE: Duration = Duration::from_millis(250);

pub struct ElevenLabsEngine {
    config: ElevenLabsRealtimeConfig,
}

impl ElevenLabsEngine {
    pub fn new(config: ElevenLabsRealtimeConfig) -> Self {
        Self { config }
    }
}

fn request(
    config: &ElevenLabsRealtimeConfig,
    hotwords: &[String],
    endpoint: &str,
) -> Result<Request<()>, String> {
    config.validate()?;
    if config.api_key.trim().is_empty() {
        return Err("请先填写 ElevenLabs API Key".into());
    }
    let mut url = url::Url::parse(endpoint).map_err(|_| "ElevenLabs 地址无效".to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("model_id", "scribe_v2_realtime")
            .append_pair("audio_format", "pcm_16000")
            .append_pair("commit_strategy", &config.commit_strategy)
            .append_pair(
                "no_verbatim",
                if config.enable_ddc { "true" } else { "false" },
            )
            .append_pair(
                "filter_background_audio",
                if config.filter_background_audio {
                    "true"
                } else {
                    "false"
                },
            );
        if !config.language.trim().is_empty() {
            query.append_pair("language_code", &config.language.trim().to_lowercase());
        }
        for code in config
            .secondary_languages
            .iter()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty())
            .collect::<BTreeSet<_>>()
        {
            query.append_pair("secondary_languages", &code);
        }
        if config.commit_strategy == "vad" {
            for (key, value) in [
                (
                    "vad_silence_threshold_secs",
                    config.vad_silence_threshold_secs.to_string(),
                ),
                ("vad_threshold", config.vad_threshold.to_string()),
                (
                    "min_speech_duration_ms",
                    config.min_speech_duration_ms.to_string(),
                ),
                (
                    "min_silence_duration_ms",
                    config.min_silence_duration_ms.to_string(),
                ),
            ] {
                query.append_pair(key, &value);
            }
        }
        if config.enable_keyterms {
            let words: BTreeSet<_> = hotwords
                .iter()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .collect();
            if words.len() > 50 {
                return Err("ElevenLabs 服务端热词最多 50 条，请减少启用词条".into());
            }
            if words.iter().any(|v| v.chars().count() > 20) {
                return Err("ElevenLabs 每条服务端热词最多 20 个字符".into());
            }
            for word in words {
                query.append_pair("keyterms", word);
            }
        }
    }
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|_| "ElevenLabs 请求构造失败".to_string())?;
    let mut key = config
        .api_key
        .trim()
        .parse::<tokio_tungstenite::tungstenite::http::HeaderValue>()
        .map_err(|_| "ElevenLabs API Key 格式无效".to_string())?;
    key.set_sensitive(true);
    req.headers_mut().insert("xi-api-key", key);
    Ok(req)
}

fn audio_message(samples: &[f32], commit: bool) -> Message {
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|sample| {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let scaled = sample * if sample < 0.0 { 32768.0 } else { 32767.0 };
            (scaled as i16).to_le_bytes()
        })
        .collect();
    Message::Text(json!({"message_type":"input_audio_chunk", "audio_base_64":STANDARD.encode(pcm), "sample_rate":16000, "commit":commit}).to_string().into())
}

#[derive(Default)]
struct Transcript {
    final_text: String,
    partial: String,
    samples: usize,
    nonzero_audio: bool,
    commits: usize,
    speech_samples: usize,
    last_speech_sent: Option<Instant>,
    confirmed_speech_samples: usize,
    last_change: Option<Instant>,
}

impl Transcript {
    fn confirmed(&self) -> bool {
        self.commits > 0
            && self.partial.is_empty()
            && self.speech_samples <= self.confirmed_speech_samples
    }
}

fn append_segment(text: &mut String, segment: &str) {
    if segment.is_empty() {
        return;
    }
    // Latin/digit word boundaries need spacing; Chinese punctuation and characters do not.
    let word = |c: char| {
        c.is_alphanumeric()
            && !matches!(c as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x323af)
    };
    if text.chars().last().is_some_and(word) && segment.chars().next().is_some_and(word) {
        text.push(' ');
    }
    text.push_str(segment);
}

fn server_error(value: &Value, api_key: &str) -> Option<(String, bool)> {
    let kind = value.get("message_type")?.as_str()?;
    let (description, fatal) = match kind {
        "auth_error" => ("API Key 无效或没有语音识别权限", true),
        "quota_exceeded" => ("额度不足，请检查账号余额", true),
        "unaccepted_terms" | "unaccepted_terms_error" => {
            ("请先在 ElevenLabs 控制台接受服务条款", true)
        }
        "invalid_request" | "input_error" | "chunk_size_exceeded" => {
            ("请求参数或音频格式无效", true)
        }
        "commit_throttled" => ("片段提交过于频繁，请稍后重试", false),
        "rate_limited" | "throttled" => ("请求过于频繁，请稍后重试", false),
        "queue_overflow" => ("音频处理队列已满，请重试", false),
        "resource_exhausted" | "transcriber_error" | "error" => {
            ("语音服务暂时不可用，请重试", false)
        }
        "session_time_limit_exceeded" => ("已达到单次会话时长限制", false),
        "insufficient_audio_activity" => ("长时间没有收到有效音频", false),
        _ => return None,
    };
    let detail = safe_detail(value["error"].as_str().unwrap_or(""), api_key);
    let suffix = if detail.is_empty() {
        String::new()
    } else {
        format!("：{detail}")
    };
    Some((
        format!("ElevenLabs：{description}（{kind}）{suffix}"),
        fatal,
    ))
}

fn safe_detail(text: &str, api_key: &str) -> String {
    static CREDENTIAL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let text = if api_key.trim().is_empty() {
        text.to_string()
    } else {
        text.replace(api_key.trim(), "[REDACTED]")
    };
    let pattern = CREDENTIAL.get_or_init(|| regex::Regex::new(
        r#"(?i)(xi-api-key|api[_-]?key|authorization|(?:access_)?token)[\s"']*[:=][\s"']*(?:bearer\s+)?[^\s,;"'}]+"#
    ).unwrap());
    pattern
        .replace_all(&text, "$1=[REDACTED]")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

/// Setup still crosses the engine boundary as a String; keep the retry decision
/// limited to known transient ElevenLabs failures, not arbitrary server details.
pub(crate) fn retryable_start_error(message: &str) -> bool {
    [
        "连接超时",
        "会话启动超时",
        "连接失败，请检查网络",
        "启动连接失败",
        "启动前断开连接",
        "启动前关闭连接",
        "心跳失败",
    ]
    .iter()
    .any(|kind| message.starts_with(&format!("ElevenLabs {kind}")))
        || [
            "rate_limited",
            "throttled",
            "commit_throttled",
            "queue_overflow",
            "resource_exhausted",
            "transcriber_error",
            "session_time_limit_exceeded",
            "insufficient_audio_activity",
            "error",
        ]
        .iter()
        .any(|kind| {
            message.starts_with("ElevenLabs：")
                && message
                    .split('）')
                    .next()
                    .is_some_and(|v| v.ends_with(&format!("（{kind}")))
        })
        || message
            .strip_prefix("ElevenLabs 连接失败：HTTP ")
            .and_then(|v| v.split('，').next())
            .and_then(|v| v.parse::<u16>().ok())
            .is_some_and(|code| code == 429 || (500..600).contains(&code))
}

struct Runtime {
    ready: AtomicBool,
    finishing: AtomicBool,
    normal_close: AtomicBool,
    api_key: String,
    updates: watch::Sender<u64>,
    transcript: Mutex<Transcript>,
    ended: watch::Sender<Option<String>>,
    events: mpsc::UnboundedSender<AsrEvent>,
}

impl Runtime {
    fn fail(&self, message: String, fatal: bool) {
        self.ready.store(false, Ordering::SeqCst);
        if self.ended.send_if_modified(|value| {
            if value.is_some() {
                return false;
            }
            *value = Some(message.clone());
            true
        }) {
            let _ = self.events.send(AsrEvent::Error { message, fatal });
        }
    }
    fn transcript(&self, value: &Value) {
        let kind = value["message_type"].as_str().unwrap_or("");
        let Some(text) = value["text"].as_str() else {
            return;
        };
        let mut state = self.transcript.lock().unwrap();
        match kind {
            "partial_transcript" => {
                if state.partial != text {
                    state.last_change = Some(Instant::now());
                }
                state.partial = text.into();
            }
            "committed_transcript" => {
                state.commits += 1;
                // A VAD result received while newer speech is still being sent
                // cannot acknowledge that newer audio. Only use a quiet boundary
                // for the already-confirmed fast path; otherwise explicitly commit.
                if state
                    .last_speech_sent
                    .is_some_and(|sent| sent.elapsed() >= FINAL_SETTLE)
                {
                    state.confirmed_speech_samples = state.speech_samples;
                }
                state.last_change = Some(Instant::now());
                append_segment(&mut state.final_text, text);
                state.partial.clear();
            }
            _ => return,
        }
        self.updates.send_modify(|revision| *revision += 1);
        let _ = self.events.send(AsrEvent::Transcript {
            final_text: state.final_text.clone(),
            partial_text: state.partial.clone(),
        });
    }
}

struct FinishTicket {
    baseline: usize,
    already_confirmed: bool,
}

enum Command {
    Audio(Vec<f32>),
    Finish(oneshot::Sender<FinishTicket>),
}

enum Control {
    Pong(tokio_tungstenite::tungstenite::Bytes),
    RetryCommit,
}

struct ElevenLabsSession {
    runtime: Arc<Runtime>,
    // Taking the sender serializes finalization against all append calls.
    sender: Mutex<Option<mpsc::UnboundedSender<Command>>>,
    final_window: Duration,
}

#[async_trait]
impl AsrEngine for ElevenLabsEngine {
    async fn create_session(
        &self,
        hotwords: &[String],
    ) -> Result<(Box<dyn AsrSession>, mpsc::UnboundedReceiver<AsrEvent>), String> {
        let req = request(&self.config, hotwords, ENDPOINT)?;
        connect_session(req, FINAL_WINDOW, self.config.api_key.clone()).await
    }
}

async fn connect_session(
    req: Request<()>,
    final_window: Duration,
    api_key: String,
) -> Result<(Box<dyn AsrSession>, mpsc::UnboundedReceiver<AsrEvent>), String> {
    let (mut socket, _) = timeout(CONNECT_TIMEOUT, connect_async(req))
        .await
        .map_err(|_| "ElevenLabs 连接超时".to_string())?
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(response) => format!(
                "ElevenLabs 连接失败：HTTP {}，请检查 API Key、权限和余额",
                response.status().as_u16()
            ),
            _ => "ElevenLabs 连接失败，请检查网络".into(),
        })?;
    let mut warnings = Vec::new();
    timeout(CONNECT_TIMEOUT, async {
        while let Some(message) = socket.next().await {
            match message.map_err(|_| "ElevenLabs 启动连接失败".to_string())? {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|_| "ElevenLabs 返回无效消息".to_string())?;
                    if let Some((message, _)) = server_error(&value, &api_key) {
                        return Err(message);
                    }
                    if value["message_type"] == "warning" {
                        warnings.push(safe_detail(
                            value["warning"].as_str().unwrap_or("服务端警告"),
                            &api_key,
                        ));
                    }
                    if value["message_type"] == "session_started" {
                        return Ok(());
                    }
                }
                Message::Close(_) => return Err("ElevenLabs 在启动前关闭连接".into()),
                Message::Ping(data) => {
                    socket
                        .send(Message::Pong(data))
                        .await
                        .map_err(|_| "ElevenLabs 心跳失败".to_string())?;
                }
                _ => {}
            }
        }
        Err("ElevenLabs 在启动前断开连接".into())
    })
    .await
    .map_err(|_| "ElevenLabs 会话启动超时".to_string())??;
    Ok(start_session(socket, final_window, api_key, warnings))
}

fn start_session<S>(
    socket: WebSocketStream<S>,
    final_window: Duration,
    api_key: String,
    warnings: Vec<String>,
) -> (Box<dyn AsrSession>, mpsc::UnboundedReceiver<AsrEvent>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (events, event_rx) = mpsc::unbounded_channel();
    let (ended, _) = watch::channel(None);
    let runtime = Arc::new(Runtime {
        ready: AtomicBool::new(true),
        finishing: AtomicBool::new(false),
        normal_close: AtomicBool::new(false),
        api_key,
        updates: watch::channel(0).0,
        transcript: Mutex::new(Transcript::default()),
        ended,
        events,
    });
    let _ = runtime.events.send(AsrEvent::Open);
    for message in warnings {
        let _ = runtime.events.send(AsrEvent::Warning { message });
    }
    let (mut sink, mut stream) = socket.split();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let writer = Arc::clone(&runtime);
    let mut writer_end = runtime.ended.subscribe();
    tokio::spawn(async move {
        let mut next_audio = Instant::now();
        let mut finished = false;
        loop {
            tokio::select! {
                biased;
                _ = writer_end.changed() => { let _ = timeout(CONNECT_TIMEOUT, sink.close()).await; break; }
                Some(control) = control_rx.recv() => {
                    let message = match control {
                        Control::Pong(data) => Message::Pong(data),
                        Control::RetryCommit => {
                            if !finished { continue; }
                            tokio::select! {
                                _ = writer_end.changed() => break,
                                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                            }
                            audio_message(&[], true)
                        }
                    };
                    let sent = tokio::select! {
                        _ = writer_end.changed() => break,
                        result = timeout(CONNECT_TIMEOUT, sink.send(message)) => result,
                    };
                    if !matches!(sent, Ok(Ok(()))) { writer.fail("ElevenLabs 控制消息发送失败，请检查网络".into(), false); break; }
                }
                command = commands.recv(), if !finished => {
                    let Some(command) = command else { break; };
                    tokio::select! {
                        _ = writer_end.changed() => { let _ = timeout(CONNECT_TIMEOUT, sink.close()).await; break; }
                        _ = sleep_until(next_audio) => {}
                    }
                    let (message, duration, ack, audio_info) = match command {
                        Command::Audio(samples) => {
                            let signal = samples.iter().any(|v| v.is_finite() && v.abs() >= 0.004);
                            let nonzero = samples.iter().any(|v| v.is_finite() && *v != 0.0);
                            (audio_message(&samples, false), Duration::from_secs_f64(samples.len() as f64 / 16000.0), None, Some((samples.len(), signal, nonzero)))
                        }
                        Command::Finish(ack) => {
                            writer.finishing.store(true, Ordering::SeqCst);
                            let state = writer.transcript.lock().unwrap();
                            let ticket = FinishTicket { baseline: state.commits, already_confirmed: state.confirmed() };
                            if state.samples == 0 || ticket.already_confirmed {
                                finished = true;
                                let _ = ack.send(ticket);
                                continue;
                            }
                            (audio_message(&[], true), Duration::ZERO, Some((ack, ticket)), None)
                        },
                    };
                    let sent = tokio::select! {
                        _ = writer_end.changed() => break,
                        result = timeout(CONNECT_TIMEOUT, sink.send(message)) => result,
                    };
                    if !matches!(sent, Ok(Ok(()))) { writer.fail("ElevenLabs 音频发送失败，请检查网络".into(), false); break; }
                    if let Some((samples, signal, nonzero)) = audio_info {
                        let mut state = writer.transcript.lock().unwrap();
                        state.samples += samples;
                        state.nonzero_audio |= nonzero;
                        if signal {
                            state.speech_samples = state.samples;
                            state.last_speech_sent = Some(Instant::now());
                        }
                    }
                    next_audio = Instant::now() + duration;
                    if let Some((ack, ticket)) = ack { finished = true; let _ = ack.send(ticket); }
                }
            }
        }
    });
    let reader = Arc::clone(&runtime);
    let mut reader_end = runtime.ended.subscribe();
    tokio::spawn(async move {
        let mut commit_retries = 0;
        loop {
            tokio::select! {
                biased;
                _ = reader_end.changed() => break,
                message = stream.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<Value>(&text) {
                                Ok(value) => {
                                    if let Some((message, fatal)) = server_error(&value, &reader.api_key) {
                                        if value["message_type"] == "commit_throttled" && commit_retries < 3 {
                                            let _ = reader.events.send(AsrEvent::Warning { message });
                                            if reader.finishing.load(Ordering::SeqCst) {
                                                commit_retries += 1;
                                                let _ = control_tx.send(Control::RetryCommit);
                                            }
                                            continue;
                                        }
                                        reader.fail(message, fatal); break;
                                    }
                                    if value["message_type"] == "warning" {
                                        let message = format!("ElevenLabs：{}", safe_detail(value["warning"].as_str().unwrap_or("服务端警告"), &reader.api_key));
                                        log_asr!(warn, "{}", message);
                                        let _ = reader.events.send(AsrEvent::Warning { message });
                                    }
                                    reader.transcript(&value);
                                }
                                Err(_) => { reader.fail("ElevenLabs 返回无效消息".into(), true); break; }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => { let _ = control_tx.send(Control::Pong(data)); }
                        Some(Ok(Message::Close(frame))) => {
                            let code = frame.as_ref().map(|v| u16::from(v.code));
                            let reason = frame.as_ref().map(|v| safe_detail(&v.reason, &reader.api_key)).unwrap_or_default();
                            if code == Some(1000) && reader.finishing.load(Ordering::SeqCst) {
                                reader.normal_close.store(true, Ordering::SeqCst);
                                reader.ready.store(false, Ordering::SeqCst);
                                reader.ended.send_replace(Some("ElevenLabs 在尾句确认前关闭连接".into()));
                            } else {
                                reader.fail(format!("ElevenLabs 连接已关闭（{}）{}", code.map(|v| v.to_string()).unwrap_or_else(|| "无状态码".into()), reason), false);
                            }
                            break;
                        }
                        None => { reader.fail("ElevenLabs 连接意外断开".into(), false); break; }
                        Some(Err(_)) => { reader.fail("ElevenLabs 连接中断，请检查网络".into(), false); break; }
                        _ => {}
                    }
                }
            }
        }
    });
    (
        Box::new(ElevenLabsSession {
            runtime,
            sender: Mutex::new(Some(sender)),
            final_window,
        }),
        event_rx,
    )
}

#[async_trait]
impl AsrSession for ElevenLabsSession {
    fn is_ready(&self) -> bool {
        self.runtime.ready.load(Ordering::SeqCst)
    }
    fn append_audio(&self, samples: &[f32]) {
        if samples.is_empty() || !self.is_ready() {
            return;
        }
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            // Keep packets within the documented 0.1–1 second recommendation, including replay.
            for chunk in samples.chunks(1600) {
                let _ = sender.send(Command::Audio(chunk.to_vec()));
            }
        }
    }
    async fn commit_and_await_final(&self) -> Result<String, String> {
        let mut ended = self.runtime.ended.subscribe();
        let mut updates = self.runtime.updates.subscribe();
        if let Some(error) = ended.borrow().clone() {
            return Err(error);
        }
        let (ack, received) = oneshot::channel();
        {
            let sender = self
                .sender
                .lock()
                .unwrap()
                .take()
                .ok_or("ElevenLabs 录音已结束")?;
            sender
                .send(Command::Finish(ack))
                .map_err(|_| "ElevenLabs 发送队列已关闭")?;
        }
        let ticket = tokio::select! {
            biased;
            result = received => result.map_err(|_| "ElevenLabs 最终提交失败")?,
            _ = ended.changed() => {
                let state = self.runtime.transcript.lock().unwrap();
                if self.runtime.normal_close.load(Ordering::SeqCst) && state.confirmed() {
                    return Ok(state.final_text.clone());
                }
                return Err(ended.borrow().clone().unwrap_or_else(|| "ElevenLabs 会话已结束".into()));
            },
        };
        let deadline = Instant::now() + self.final_window;
        loop {
            if let Some(error) = ended.borrow().clone() {
                if !self.runtime.normal_close.load(Ordering::SeqCst) {
                    return Err(error);
                }
            }
            let (complete, quiet_until) = {
                let state = self.runtime.transcript.lock().unwrap();
                if state.samples == 0 {
                    return Ok(String::new());
                }
                let confirmed = state.partial.is_empty()
                    && (ticket.already_confirmed || state.commits > ticket.baseline);
                let silent = !state.nonzero_audio && state.partial.is_empty();
                let quiet_until = state.last_change.unwrap_or_else(Instant::now) + FINAL_SETTLE;
                if confirmed
                    && (Instant::now() >= quiet_until
                        || self.runtime.normal_close.load(Ordering::SeqCst))
                {
                    return Ok(state.final_text.clone());
                }
                (
                    confirmed || silent,
                    if silent && state.last_change.is_none() {
                        deadline.min(Instant::now() + FINAL_SETTLE)
                    } else {
                        quiet_until
                    },
                )
            };
            if let Some(error) = ended.borrow().clone() {
                return Err(error);
            }
            tokio::select! {
                biased;
                _ = ended.changed() => {},
                _ = updates.changed() => {},
                _ = sleep_until(deadline) => return Err("ElevenLabs 最终结果超时，请检查网络后重试".into()),
                _ = sleep_until(quiet_until), if complete => {
                    let state = self.runtime.transcript.lock().unwrap();
                    if !state.nonzero_audio && state.partial.is_empty() { return Ok(state.final_text.clone()); }
                },
            }
        }
    }

    fn recognized_text(&self) -> Option<String> {
        let state = self.runtime.transcript.lock().unwrap();
        let mut text = state.final_text.clone();
        append_segment(&mut text, &state.partial);
        Some(text)
    }

    fn close(&self) {
        self.runtime.normal_close.store(false, Ordering::SeqCst);
        self.runtime.ready.store(false, Ordering::SeqCst);
        self.sender.lock().unwrap().take();
        self.runtime
            .ended
            .send_replace(Some("ElevenLabs 会话已取消".into()));
    }
}

impl Drop for ElevenLabsSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::accept_async;

    fn config() -> ElevenLabsRealtimeConfig {
        ElevenLabsRealtimeConfig {
            api_key: "test-secret".into(),
            ..Default::default()
        }
    }

    fn params(config: &ElevenLabsRealtimeConfig, words: &[String]) -> Vec<(String, String)> {
        let req = request(config, words, ENDPOINT).unwrap();
        url::Url::parse(&req.uri().to_string())
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    #[test]
    fn request_maps_shared_fields_and_keeps_key_out_of_url() {
        let mut config = config();
        config.language = " ZH ".into();
        config.secondary_languages = vec![" EN ".into(), "en".into(), "".into()];
        config.enable_ddc = true;
        let req = request(&config, &[], ENDPOINT).unwrap();
        assert_eq!(req.headers()["xi-api-key"], "test-secret");
        assert!(req.headers()["xi-api-key"].is_sensitive());
        assert!(!req.uri().to_string().contains("test-secret"));
        let pairs = params(&config, &[]);
        assert!(pairs.contains(&("language_code".into(), "zh".into())));
        assert!(pairs.contains(&("no_verbatim".into(), "true".into())));
        assert_eq!(
            pairs
                .iter()
                .filter(|(k, _)| k == "secondary_languages")
                .count(),
            1
        );
        assert!(!params(&self::config(), &[])
            .iter()
            .any(|(k, _)| k == "language_code"));
    }

    #[test]
    fn defaults_and_validation_do_not_silently_fall_back() {
        let config: ElevenLabsRealtimeConfig = serde_json::from_value(json!({})).unwrap();
        assert_eq!(config.commit_strategy, "vad");
        assert_eq!(config.vad_threshold, 0.4);
        assert!(!config.enable_keyterms);
        for patch in [
            json!({"vad_threshold":0.0}),
            json!({"vad_silence_threshold_secs":3.1}),
            json!({"min_speech_duration_ms":49}),
            json!({"min_silence_duration_ms":2001}),
            json!({"language":"zh-CN"}),
            json!({"commit_strategy":"turn_prediction"}),
        ] {
            let bad: ElevenLabsRealtimeConfig = serde_json::from_value(patch).unwrap();
            assert!(bad.validate().is_err());
        }
        assert!(serde_json::from_value::<ElevenLabsRealtimeConfig>(
            json!({"min_speech_duration_ms":100.5})
        )
        .is_err());
        assert!(serde_json::from_value::<ElevenLabsRealtimeConfig>(
            json!({"secondary_languages":"en"})
        )
        .is_err());
    }

    #[test]
    fn keyterms_are_opt_in_deduplicated_and_bounded() {
        let mut config = config();
        let words: Vec<_> = (0..51).map(|i| format!("word{i}")).collect();
        assert!(!params(&config, &words).iter().any(|(k, _)| k == "keyterms"));
        config.enable_keyterms = true;
        assert!(request(&config, &words, ENDPOINT).is_err());
        assert!(request(&config, &["x".repeat(21)], ENDPOINT).is_err());
        let pairs = params(&config, &[" A&B ".into(), "A&B".into(), "中文".into()]);
        assert_eq!(pairs.iter().filter(|(k, _)| k == "keyterms").count(), 2);
        assert!(pairs.contains(&("keyterms".into(), "A&B".into())));
        config.commit_strategy = "manual".into();
        assert!(!params(&config, &[])
            .iter()
            .any(|(k, _)| k.starts_with("vad_") || k.ends_with("_duration_ms")));
    }

    #[test]
    fn pcm_encoding_clamps_and_handles_invalid_samples() {
        let message = audio_message(&[-2.0, 0.0, 2.0, f32::NAN], false);
        let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        let bytes = STANDARD
            .decode(value["audio_base_64"].as_str().unwrap())
            .unwrap();
        assert_eq!(bytes, [0, 128, 0, 0, 255, 127, 0, 0]);
        assert_eq!(value["sample_rate"], 16000);
        assert_eq!(value["commit"], false);
    }

    #[test]
    fn segments_keep_language_boundaries_and_existing_whitespace() {
        for (segments, expected) in [
            (vec!["hello", "world", "!"], "hello world!"),
            (vec!["你好", "世界", "。"], "你好世界。"),
            (vec!["hello ", " world"], "hello  world"),
            (vec!["Zażółć", "gęślą"], "Zażółć gęślą"),
        ] {
            let mut text = String::new();
            for segment in segments {
                append_segment(&mut text, segment);
            }
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn errors_are_classified_without_echoing_secrets() {
        for (kind, fatal) in [
            ("auth_error", true),
            ("quota_exceeded", true),
            ("invalid_request", true),
            ("rate_limited", false),
            ("queue_overflow", false),
            ("session_time_limit_exceeded", false),
        ] {
            let (message, actual) = server_error(
                &json!({"message_type":kind,"error":"test-secret"}),
                "test-secret",
            )
            .unwrap();
            assert_eq!(actual, fatal);
            assert!(!message.contains("test-secret"));
        }
        assert!(server_error(&json!({"message_type":"warning"}), "test-secret").is_none());
    }

    type Server = WebSocketStream<TcpStream>;
    async fn mock() -> (String, tokio::task::JoinHandle<Server>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            accept_async(tcp).await.unwrap()
        });
        (endpoint, server)
    }
    async fn send(server: &mut Server, kind: &str, text: &str) {
        server
            .send(Message::Text(
                json!({"message_type":kind,"text":text}).to_string().into(),
            ))
            .await
            .unwrap();
    }
    async fn session() -> (
        Arc<dyn AsrSession>,
        mpsc::UnboundedReceiver<AsrEvent>,
        Server,
    ) {
        let (endpoint, server) = mock().await;
        let client = tokio::spawn(connect_session(
            request(&config(), &[], &endpoint).unwrap(),
            Duration::from_millis(1000),
            "test-secret".into(),
        ));
        let mut server = server.await.unwrap();
        send(&mut server, "session_started", "").await;
        let (session, events) = client.await.unwrap().unwrap();
        (Arc::from(session), events, server)
    }
    async fn next_audio(server: &mut Server) -> Value {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(Ok(Message::Text(text))) = server.next().await {
                    return serde_json::from_str(&text).unwrap();
                }
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn waits_for_session_started_and_rejects_startup_error() {
        let (endpoint, server) = mock().await;
        let client = tokio::spawn(connect_session(
            request(&config(), &[], &endpoint).unwrap(),
            Duration::ZERO,
            "test-secret".into(),
        ));
        let mut server = server.await.unwrap();
        assert!(!client.is_finished());
        send(&mut server, "auth_error", "").await;
        assert!(client.await.unwrap().err().unwrap().contains("API Key"));
    }

    #[tokio::test]
    async fn fifo_pacing_and_late_tail_do_not_resolve_on_old_vad_commit() {
        let (session, mut events, mut server) = session().await;
        assert!(matches!(events.recv().await, Some(AsrEvent::Open)));
        session.append_audio(&vec![0.1; 3200]);
        let finalizing = Arc::clone(&session);
        let result = tokio::spawn(async move { finalizing.commit_and_await_final().await });
        let first = next_audio(&mut server).await;
        let sent_at = Instant::now();
        let second = next_audio(&mut server).await;
        assert!(sent_at.elapsed() >= Duration::from_millis(85));
        assert_eq!(first["commit"], false);
        assert_eq!(second["commit"], false);
        assert_eq!(next_audio(&mut server).await["commit"], true);
        send(&mut server, "committed_transcript", "hello").await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!result.is_finished());
        send(&mut server, "partial_transcript", "wor").await;
        send(&mut server, "partial_transcript", "world").await;
        send(&mut server, "committed_transcript", "world").await;
        send(&mut server, "partial_transcript", "").await;
        assert_eq!(
            timeout(Duration::from_millis(700), result)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "hello world"
        );
        session.close();
    }

    #[tokio::test]
    async fn silence_and_no_audio_finish_without_a_transcript() {
        for (audio, empty_commit) in [
            (vec![], false),
            (vec![0.0; 1600], false),
            (vec![0.1; 1600], true),
        ] {
            let (session, _events, mut server) = session().await;
            session.append_audio(&audio);
            let copy = Arc::clone(&session);
            let result = tokio::spawn(async move { copy.commit_and_await_final().await });
            if !audio.is_empty() {
                assert_eq!(next_audio(&mut server).await["commit"], false);
            }
            if !audio.is_empty() {
                assert_eq!(next_audio(&mut server).await["commit"], true);
            }
            if empty_commit {
                send(&mut server, "committed_transcript", "").await;
            }
            assert_eq!(result.await.unwrap().unwrap(), "");
            session.close();
        }
    }

    #[tokio::test]
    async fn unfinished_partial_is_a_timeout_and_close_interrupts_wait() {
        let (session, _events, mut server) = session().await;
        session.append_audio(&[0.1; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        next_audio(&mut server).await;
        next_audio(&mut server).await;
        send(&mut server, "partial_transcript", "tail").await;
        assert!(result.await.unwrap().unwrap_err().contains("超时"));
        session.close();

        let (session, _events, mut server) = self::session().await;
        session.append_audio(&[0.1; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        next_audio(&mut server).await;
        next_audio(&mut server).await;
        session.close();
        assert!(timeout(Duration::from_millis(100), result)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(!session.is_ready());
    }

    #[tokio::test]
    async fn warnings_and_keepalives_are_not_errors_and_disconnect_is_reported() {
        let (session, mut events, mut server) = session().await;
        events.recv().await;
        server
            .send(Message::Text(
                json!({"message_type":"warning","warning":"test-secret is not retained"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let Some(AsrEvent::Warning { message }) = events.recv().await else {
            panic!("expected warning")
        };
        assert!(message.contains("not retained"));
        assert!(!message.contains("test-secret"));
        assert!(session.is_ready());
        for text in ["hello", "hello", ""] {
            send(&mut server, "partial_transcript", text).await;
            let Some(AsrEvent::Transcript {
                final_text,
                partial_text,
            }) = events.recv().await
            else {
                panic!("expected transcript")
            };
            assert_eq!(final_text, "");
            assert_eq!(partial_text, text);
        }
        server.close(None).await.unwrap();
        assert!(matches!(
            events.recv().await,
            Some(AsrEvent::Error { fatal: false, .. })
        ));
        assert!(!session.is_ready());
    }

    #[test]
    fn all_protocol_errors_keep_safe_reasons_and_setup_retry_policy() {
        for (kind, fatal) in [
            ("error", false),
            ("auth_error", true),
            ("quota_exceeded", true),
            ("commit_throttled", false),
            ("unaccepted_terms", true),
            ("rate_limited", false),
            ("queue_overflow", false),
            ("resource_exhausted", false),
            ("session_time_limit_exceeded", false),
            ("input_error", true),
            ("invalid_request", true),
            ("chunk_size_exceeded", true),
            ("insufficient_audio_activity", false),
            ("transcriber_error", false),
        ] {
            let (message, actual) = server_error(&json!({"message_type":kind,"error":"specific reason; xi-api-key=test-secret; token=other-secret"}), "test-secret").unwrap();
            assert_eq!(actual, fatal);
            assert!(message.contains("specific reason"));
            assert!(!message.contains("test-secret"));
            assert!(!message.contains("other-secret"));
            assert_eq!(retryable_start_error(&message), !fatal);
        }
        for message in [
            "请先填写 ElevenLabs API Key",
            "ElevenLabs 参数格式错误，请检查模型配置",
            "ElevenLabs 连接失败：HTTP 401，请检查 API Key、权限和余额",
        ] {
            assert!(!retryable_start_error(message));
        }
        for message in [
            "ElevenLabs 连接超时",
            "ElevenLabs 会话启动超时",
            "ElevenLabs 连接失败：HTTP 503，请检查 API Key、权限和余额",
        ] {
            assert!(retryable_start_error(message));
        }
    }

    #[tokio::test]
    async fn already_confirmed_vad_text_finishes_without_a_duplicate_commit() {
        let (session, mut events, mut server) = session().await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        next_audio(&mut server).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        send(&mut server, "committed_transcript", "已经确认").await;
        events.recv().await;
        session.append_audio(&[0.0; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        assert_eq!(next_audio(&mut server).await["commit"], false);
        assert_eq!(
            timeout(Duration::from_millis(600), result)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "已经确认"
        );
        assert!(timeout(Duration::from_millis(30), server.next())
            .await
            .is_err());
        session.close();
    }

    #[tokio::test]
    async fn new_speech_after_a_vad_commit_still_waits_for_its_tail() {
        let (session, mut events, mut server) = session().await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        next_audio(&mut server).await;
        send(&mut server, "committed_transcript", "first").await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        next_audio(&mut server).await;
        assert_eq!(next_audio(&mut server).await["commit"], true);
        send(&mut server, "partial_transcript", "second").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!result.is_finished());
        send(&mut server, "committed_transcript", "second").await;
        assert_eq!(
            timeout(Duration::from_millis(600), result)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "first second"
        );
        session.close();
    }

    #[tokio::test]
    async fn final_commit_throttling_retries_on_the_same_connection() {
        let (session, mut events, mut server) = session().await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        next_audio(&mut server).await;
        assert_eq!(next_audio(&mut server).await["commit"], true);
        server
            .send(Message::Text(
                json!({"message_type":"commit_throttled","error":"Please commit less often"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert!(matches!(
            events.recv().await,
            Some(AsrEvent::Warning { .. })
        ));
        assert!(session.is_ready());
        assert_eq!(next_audio(&mut server).await["commit"], true);
        send(&mut server, "committed_transcript", "done").await;
        assert_eq!(result.await.unwrap().unwrap(), "done");
        session.close();
    }

    #[tokio::test]
    async fn normal_close_after_final_text_succeeds_but_incomplete_close_fails() {
        for confirmed in [true, false] {
            let (session, _events, mut server) = session().await;
            session.append_audio(&[0.1; 1600]);
            let copy = Arc::clone(&session);
            let result = tokio::spawn(async move { copy.commit_and_await_final().await });
            next_audio(&mut server).await;
            next_audio(&mut server).await;
            send(
                &mut server,
                if confirmed {
                    "committed_transcript"
                } else {
                    "partial_transcript"
                },
                "recognized",
            )
            .await;
            server
                .close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code:
                        tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: "Finished".into(),
                }))
                .await
                .unwrap();
            let text = result.await.unwrap();
            if confirmed {
                assert_eq!(text.unwrap(), "recognized");
            } else {
                assert!(text.is_err());
                assert_eq!(session.recognized_text().as_deref(), Some("recognized"));
            }
            session.close();
        }
    }

    #[tokio::test]
    async fn confirmed_text_is_available_when_an_error_interrupts_finalization() {
        let (session, mut events, mut server) = session().await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        next_audio(&mut server).await;
        send(&mut server, "committed_transcript", "saved").await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        next_audio(&mut server).await;
        next_audio(&mut server).await;
        server
            .send(Message::Text(
                json!({"message_type":"quota_exceeded","error":"No credits left"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert!(result
            .await
            .unwrap()
            .unwrap_err()
            .contains("No credits left"));
        assert_eq!(session.recognized_text().as_deref(), Some("saved"));
        session.close();
    }
    #[tokio::test]
    async fn quiet_audio_is_not_mistaken_for_digital_silence() {
        let (session, _events, mut server) = session().await;
        session.append_audio(&[0.001; 1600]);
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        next_audio(&mut server).await;
        next_audio(&mut server).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!result.is_finished());
        send(&mut server, "committed_transcript", "quiet voice").await;
        assert_eq!(result.await.unwrap().unwrap(), "quiet voice");
        session.close();
    }
    #[tokio::test]
    async fn delayed_old_vad_commit_does_not_acknowledge_newly_sent_speech() {
        let (session, mut events, mut server) = session().await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        next_audio(&mut server).await;
        send(&mut server, "partial_transcript", "first").await;
        events.recv().await;
        session.append_audio(&[0.1; 1600]);
        next_audio(&mut server).await;
        send(&mut server, "committed_transcript", "first").await;
        events.recv().await;
        let copy = Arc::clone(&session);
        let result = tokio::spawn(async move { copy.commit_and_await_final().await });
        assert_eq!(next_audio(&mut server).await["commit"], true);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!result.is_finished());
        send(&mut server, "committed_transcript", "second").await;
        assert_eq!(result.await.unwrap().unwrap(), "first second");
        session.close();
    }
}
