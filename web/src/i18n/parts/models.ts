/**
 * Audio / language model translations: registry names, descriptions, tags and
 * the data-driven field metadata in `settings/lib/model.ts`.
 *
 * Keys are the exact Chinese strings the registry and field metadata carry, so
 * the same table serves both the model cards and the parameter forms.
 */
export const EN_MODELS: Record<string, string> = {
  // ── Registry: model names ──────────────────────────────────────────
  "火山引擎 - 豆包流式语音识别模型2.0": "Volcengine — Doubao Streaming ASR 2.0",
  "ElevenLabs - Scribe v2 Realtime": "ElevenLabs — Scribe v2 Realtime",
  "Zipformer 流式中文": "Zipformer Streaming (Chinese)",
  "Zipformer 流式中英双语": "Zipformer Streaming (Chinese + English)",
  "SenseVoice 多语言": "SenseVoice Multilingual",
  "FunASR-Nano 中英日三语": "FunASR-Nano (Chinese / English / Japanese)",
  "Qwen3 ASR 多语言": "Qwen3 ASR Multilingual",
  "Parakeet TDT 0.6B v3（英语等 25 种语言）": "Parakeet TDT 0.6B v3 (English + 24 more)",
  "Parakeet TDT 0.6B v2（英语）": "Parakeet TDT 0.6B v2 (English)",
  "Silero VAD": "Silero VAD",
  "CT-Transformer 标点恢复": "CT-Transformer Punctuation",
  基础模型: "Base model",

  // ── Registry: descriptions ─────────────────────────────────────────
  "¥1 / 小时，中文用户首选": "¥1 per hour — the default choice for Chinese",
  "$0.39 / 小时": "$0.39 per hour",
  "实时流式本地模型，中文高精度低延迟识别":
    "Real-time streaming local model — accurate, low-latency Chinese recognition",
  "实时流式本地模型，支持中英双语低延迟识别":
    "Real-time streaming local model — low-latency Chinese and English",
  "高精度离线模型，支持中英日韩粤 5种语言":
    "High-accuracy offline model supporting Chinese, English, Japanese, Korean and Cantonese",
  "基于 FunASR-Nano-2512 的高精度离线模型，支持中英日三语、多种方言及歌词识别":
    "High-accuracy offline model built on FunASR-Nano-2512: three languages, many dialects, lyrics recognition",
  "基于 Qwen3-ASR-0.6B 的多语言语音识别模型，支持31种语言及20种中文方言、歌词/说唱识别":
    "Multilingual model built on Qwen3-ASR-0.6B: 31 languages, 20 Chinese dialects, lyrics and rap",
  "NVIDIA Parakeet TDT 0.6B v3 的 int8 量化 ONNX 版本，英语识别快、自带大小写与标点，额外支持 24 种欧洲语言":
    "int8-quantised ONNX build of NVIDIA Parakeet TDT 0.6B v3 — fast English recognition with built-in casing and punctuation, plus 24 European languages",
  "NVIDIA Parakeet TDT 0.6B v2 的 int8 量化 ONNX 版本，英语识别快、自带大小写与标点":
    "int8-quantised ONNX build of NVIDIA Parakeet TDT 0.6B v2 — fast English recognition with built-in casing and punctuation",
  "语音活动检测模型，语音分段的基础依赖":
    "Voice activity detection — the base dependency for speech segmentation",
  "中英双语标点恢复模型，为不支持自动标点的 ASR 模型添加标点符号，下载后自动生效":
    "Chinese/English punctuation restoration for ASR models without it; takes effect once downloaded",

  // ── Registry: tags ─────────────────────────────────────────────────
  流式输出: "Streaming",
  免费试用额度: "Free trial quota",
  热词库: "Hotwords",
  "中英文,方言": "Chinese + English, dialects",
  "中英日, 方言": "Chinese/English/Japanese, dialects",
  按量计费: "Pay as you go",
  中文识别率高: "High Chinese accuracy",
  中文识别高: "Strong Chinese",
  中英混说: "Code-switching",
  歌词识别: "Lyrics",
  多语言: "Multilingual",
  "90+多语言": "90+ languages",
  英语优先: "English first",
  "25种欧洲语言": "25 European languages",
  自带标点: "Built-in punctuation",
  低延时: "Low latency",
  低内存占用: "Low memory",
  实时流式本地模型: "Real-time local streaming",

  // ── Field metadata: labels ─────────────────────────────────────────
  模拟流式输出: "Simulated streaming",
  热词强化: "Hotword boost",
  热词替换: "Hotword replacement",
  线程数: "Threads",
  推理后端: "Inference backend",
  标点符号: "Punctuation",
  "VAD 阈值": "VAD threshold",
  "VAD 最小静音时长": "VAD minimum silence",
  "VAD 最小说话时长": "VAD minimum speech",
  "VAD 最大说话时长": "VAD maximum speech",
  鉴权方式: "Auth mode",
  旧版: "Legacy",
  新版: "v2",
  "WebSocket 地址": "WebSocket URL",
  "输入 App ID": "Enter App ID",
  "输入 Access Token": "Enter access token",
  "输入 Resource ID": "Enter resource ID",
  "输入 API Key": "Enter API key",
  二遍识别: "Second-pass recognition",
  首字加速: "First-token acceleration",
  语义顺滑: "Disfluency removal",
  数字格式化: "Number formatting",
  自动标点: "Automatic punctuation",
  "热词表 ID": "Hotword list ID",
  "输入热词表 ID": "Enter hotword list ID",
  "替换词表 ID": "Replacement list ID",
  "输入替换词表 ID": "Enter replacement list ID",
  语言: "Language",
  留空则自动检测: "Leave empty to auto-detect",
  音频采样: "Sample rate",
  声道: "Channels",
  音频格式: "Audio format",
  采样位数: "Bit depth",
  模型名称: "Model name",
  模型版本: "Model version",
  操作类型: "Operation",
  请求序号: "Sequence number",
  显示分句: "Return utterances",
  结果返回方式: "Result mode",
  全量: "Full",
  增量: "Incremental",
  首字加速率: "First-token acceleration",
  "SSD 大模型版本": "SSM model version",
  "200（大模型 SSD）": "200 (large SSM)",
  "100（标准）": "100 (standard)",
  繁体输出: "Traditional output",
  不转换: "No conversion",
  "繁体（大陆）": "Traditional (mainland)",
  台湾正体: "Taiwan standard",
  香港繁体: "Hong Kong traditional",
  "强制判停时长 (ms)": "Forced endpoint (ms)",
  "最短强制判停 (ms)": "Minimum forced endpoint (ms)",
  "语义切句静音阈值 (ms)": "Semantic split silence (ms)",
  "数字格式化 (ITN)": "Number formatting (ITN)",
  系统提示词: "System prompt",
  用户提示词: "User prompt",
  "最大生成 Token": "Max generated tokens",
  采样温度: "Temperature",
  随机种子: "Random seed",
  最大活跃路径: "Max active paths",
  建模单元: "Modelling unit",
  最大总长度: "Max total length",
  端点检测: "Endpoint detection",
  规则1最小尾静音: "Rule 1 minimum trailing silence",
  规则2最小尾静音: "Rule 2 minimum trailing silence",
  规则3最短话语长度: "Rule 3 minimum utterance length",
  文本确认方式: "Commit mode",
  手动提交: "Manual commit",
  VAD分段: "VAD segmentation",
  第二语言: "Secondary language",
  "例如 zh, en": "e.g. zh, en",
  过滤背景语音: "Filter background speech",
  转写编辑: "Transcript editing",
  "例如：将日期统一写成 YYYY-MM-DD 格式": "e.g. normalise dates to YYYY-MM-DD",
  "分段静音时长（秒）": "Segment silence (s)",
  自适应: "Adaptive",
  自动: "Auto",
  关闭: "Off",
  开启: "On",
  禁用: "Disabled",
  强制启用: "Forced",

  // ── Field metadata: placeholders and hints ─────────────────────────
  "留空自动识别；例如 zh、en、pol": "Leave empty to detect; e.g. zh, en, pl",
  "可能混用的语言代码，以逗号分隔；留空不限制":
    "Comma-separated languages that may be mixed in; leave empty for no limit",
  "使用现有启用的热词；最多 50 条，每条最多 20 个字符":
    "Uses your enabled hotwords; up to 50 entries, 20 characters each",
  "开启后基础转写费用增加 20%": "Raises base transcription cost by 20% when enabled",
  "去除口头填充、重复起句和不流畅表达；不影响临时文字的实时显示":
    "Removes filler words, restarted sentences and disfluencies; live partial text is unaffected",
  "控制每段识别文字何时被确认，不影响实时显示。手动提交：结束录音时提交当前片段；VAD分段：服务端根据停顿自动确认片段。两种方式在结束录音后，都会等待最终结果再输出。":
    "Controls when a segment of recognized text is committed; live display is unaffected. Manual commits the current segment when recording stops; VAD segmentation commits automatically at pauses. Both wait for the final result before output.",
  减少附近谈话与环境噪声造成的误识别: "Fewer false recognitions from nearby talk and ambient noise",
  "留空关闭；最多 2000 个字符。服务端编辑每个确认片段，最终输出等待编辑结果；编辑缺失时提示失败并允许重试。":
    "Leave empty to disable; up to 2000 characters. The server edits each committed segment and the final output waits for the edit; a missing edit reports failure and allows a retry.",
  "填写后基础转写费用增加 30%，每个确认片段至少按 10 秒计费":
    "Raises base transcription cost by 30%; each committed segment bills at least 10 seconds",
  "停顿多久后确认当前片段；0.3–3 秒": "How long a pause commits the current segment; 0.3–3 s",
  "0.1–0.9，越低越容易检测到讲话；仅影响服务端 VAD":
    "0.1–0.9; lower detects speech more readily; server-side VAD only",
  "过滤过短声音；50–2000 ms": "Filters very short sounds; 50–2000 ms",
  "判定语音间断的最小静音；50–2000 ms，与分段静音时长不同":
    "Minimum silence that ends an utterance; 50–2000 ms, distinct from segment silence",
};
