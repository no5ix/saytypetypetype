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
    signal: bool,
    commits: usize,
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

fn server_error(value: &Value) -> Option<(String, bool)> {
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
        "rate_limited" | "throttled" | "commit_throttled" => ("请求过于频繁，请稍后重试", false),
        "queue_overflow" => ("音频处理队列已满，请重试", false),
        "resource_exhausted" | "transcriber_error" | "error" => {
            ("语音服务暂时不可用，请重试", false)
        }
        "session_time_limit_exceeded" => ("已达到单次会话时长限制", false),
        "insufficient_audio_activity" => ("长时间没有收到有效音频", false),
        _ => return None,
    };
    // Do not echo arbitrary server payloads: they can contain request credentials.
    Some((format!("ElevenLabs：{description}（{kind}）"), fatal))
}

struct Runtime {
    ready: AtomicBool,
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
            "partial_transcript" => state.partial = text.into(),
            "committed_transcript" => {
                state.commits += 1;
                append_segment(&mut state.final_text, text);
                state.partial.clear();
            }
            _ => return,
        }
        let _ = self.events.send(AsrEvent::Transcript {
            final_text: state.final_text.clone(),
            partial_text: state.partial.clone(),
        });
    }
}

enum Command {
    Audio(Vec<f32>),
    Finish(oneshot::Sender<()>),
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
        connect_session(req, FINAL_WINDOW).await
    }
}

async fn connect_session(
    req: Request<()>,
    final_window: Duration,
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
    timeout(CONNECT_TIMEOUT, async {
        while let Some(message) = socket.next().await {
            match message.map_err(|_| "ElevenLabs 启动连接失败".to_string())? {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|_| "ElevenLabs 返回无效消息".to_string())?;
                    if let Some((message, _)) = server_error(&value) {
                        return Err(message);
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
    Ok(start_session(socket, final_window))
}

fn start_session<S>(
    socket: WebSocketStream<S>,
    final_window: Duration,
) -> (Box<dyn AsrSession>, mpsc::UnboundedReceiver<AsrEvent>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (events, event_rx) = mpsc::unbounded_channel();
    let (ended, _) = watch::channel(None);
    let runtime = Arc::new(Runtime {
        ready: AtomicBool::new(true),
        transcript: Mutex::new(Transcript::default()),
        ended,
        events,
    });
    let _ = runtime.events.send(AsrEvent::Open);
    let (mut sink, mut stream) = socket.split();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let (pong_tx, mut pong_rx) = mpsc::unbounded_channel();
    let writer = Arc::clone(&runtime);
    let mut writer_end = runtime.ended.subscribe();
    tokio::spawn(async move {
        let mut next_audio = Instant::now();
        let mut finished = false;
        loop {
            tokio::select! {
                biased;
                _ = writer_end.changed() => { let _ = timeout(CONNECT_TIMEOUT, sink.close()).await; break; }
                Some(data) = pong_rx.recv() => {
                    if !matches!(timeout(CONNECT_TIMEOUT, sink.send(Message::Pong(data))).await, Ok(Ok(()))) { writer.fail("ElevenLabs 心跳发送失败".into(), false); break; }
                }
                command = commands.recv(), if !finished => {
                    let Some(command) = command else { break; };
                    tokio::select! {
                        _ = writer_end.changed() => { let _ = timeout(CONNECT_TIMEOUT, sink.close()).await; break; }
                        _ = sleep_until(next_audio) => {}
                    }
                    let (message, duration, ack) = match command {
                        Command::Audio(samples) => {
                            let mut state = writer.transcript.lock().unwrap();
                            state.samples += samples.len();
                            state.signal |= samples.iter().any(|v| v.is_finite() && v.abs() >= 0.004);
                            (audio_message(&samples, false), Duration::from_secs_f64(samples.len() as f64 / 16000.0), None)
                        }
                        Command::Finish(ack) => (audio_message(&[], true), Duration::ZERO, Some(ack)),
                    };
                    let sent = tokio::select! {
                        _ = writer_end.changed() => break,
                        result = timeout(CONNECT_TIMEOUT, sink.send(message)) => result,
                    };
                    if !matches!(sent, Ok(Ok(()))) { writer.fail("ElevenLabs 音频发送失败，请检查网络".into(), false); break; }
                    next_audio = Instant::now() + duration;
                    if let Some(ack) = ack { finished = true; let _ = ack.send(()); }
                }
            }
        }
    });
    let reader = Arc::clone(&runtime);
    let mut reader_end = runtime.ended.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = reader_end.changed() => break,
                message = stream.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<Value>(&text) {
                                Ok(value) => {
                                    if let Some((message, fatal)) = server_error(&value) { reader.fail(message, fatal); break; }
                                    if value["message_type"] == "warning" { log_asr!(warn, "ElevenLabs returned a non-fatal warning"); }
                                    reader.transcript(&value);
                                }
                                Err(_) => { reader.fail("ElevenLabs 返回无效消息".into(), true); break; }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => { let _ = pong_tx.send(data); }
                        Some(Ok(Message::Close(_))) | None => { reader.fail("ElevenLabs 连接已关闭".into(), false); break; }
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
        tokio::select! {
            _ = ended.changed() => return Err(ended.borrow().clone().unwrap_or_else(|| "ElevenLabs 会话已结束".into())),
            result = received => { result.map_err(|_| "ElevenLabs 最终提交失败")?; }
        }
        if self.runtime.transcript.lock().unwrap().samples == 0 {
            return Ok(String::new());
        }
        // The API has no commit ID or end-of-stream acknowledgement. An older VAD
        // result can arrive after our commit. Collect the entire bounded window
        // rather than incorrectly resolving on the first committed event.
        tokio::select! {
            _ = ended.changed() => return Err(ended.borrow().clone().unwrap_or_else(|| "ElevenLabs 会话已结束".into())),
            _ = tokio::time::sleep(self.final_window) => {}
        }
        let state = self.runtime.transcript.lock().unwrap();
        if !state.partial.is_empty()
            || (state.final_text.is_empty() && state.signal && state.commits == 0)
        {
            return Err("ElevenLabs 最终结果超时，请检查网络后重试".into());
        }
        Ok(state.final_text.clone())
    }
    fn close(&self) {
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
            let (message, actual) =
                server_error(&json!({"message_type":kind,"error":"test-secret"})).unwrap();
            assert_eq!(actual, fatal);
            assert!(!message.contains("test-secret"));
        }
        assert!(server_error(&json!({"message_type":"warning"})).is_none());
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
            Duration::from_millis(180),
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
        assert_eq!(result.await.unwrap().unwrap(), "hello world");
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
            assert_eq!(next_audio(&mut server).await["commit"], true);
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
        send(&mut server, "warning", "").await;
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
}
