import Link from "next/link"
import type { Locale } from "@/lib/i18n"

const asrModels = [
  {
    name: ["豆包流式识别 2.0", "Doubao Streaming ASR 2.0"],
    online: true,
    feature: [
      "中英混说；方言支持随接口模式而异",
      "Chinese–English speech; dialect support varies by API mode",
    ],
    streaming: true,
    hotwords: true,
  },
  {
    name: ["ElevenLabs Scribe v2 Realtime", "ElevenLabs Scribe v2 Realtime"],
    online: true,
    feature: [
      "高精度多语言转写，自动识别语言",
      "Accurate multilingual transcription with automatic language detection",
    ],
    streaming: true,
    hotwords: true,
  },
  {
    name: ["SenseVoice INT8", "SenseVoice INT8"],
    online: false,
    feature: [
      "轻量量化模型，支持中英日韩粤",
      "Compact quantized model for Chinese, English, Japanese, Korean and Cantonese",
    ],
    streaming: false,
    hotwords: false,
  },
  {
    name: ["FunASR-Nano INT8", "FunASR-Nano INT8"],
    online: false,
    feature: [
      "中英日识别，支持中文方言与歌词",
      "Chinese, English and Japanese; Chinese dialects and singing voice",
    ],
    streaming: false,
    hotwords: true,
  },
  {
    name: ["Zipformer 中英双语", "Zipformer Chinese–English"],
    online: false,
    feature: [
      "原生流式，中英混说，低延迟",
      "Native streaming, mixed Chinese–English speech and low latency",
    ],
    streaming: true,
    hotwords: true,
  },
  {
    name: ["Zipformer 中文 INT8", "Zipformer Chinese INT8"],
    online: false,
    feature: [
      "原生流式，轻量中文量化模型",
      "Native streaming with a compact, quantized Chinese model",
    ],
    streaming: true,
    hotwords: true,
  },
  {
    name: ["Qwen3-ASR 0.6B INT8", "Qwen3-ASR 0.6B INT8"],
    online: false,
    feature: [
      "多语言、中文方言，支持歌词识别",
      "Multilingual speech, Chinese dialects and singing voice",
    ],
    streaming: false,
    hotwords: true,
  },
] as const
const llmServices = [
  { name: "DeepSeek", online: true },
  { name: "OpenAI", online: true },
  { name: "Gemini", online: true },
  { name: "Anthropic", online: true },
  { name: "硅基流动 / SiliconFlow", online: true },
  { name: "OpenRouter", online: true },
  { name: "Ollama", online: false },
] as const

export function ModelTables({
  locale,
  detailed = false,
}: {
  locale: Locale
  detailed?: boolean
}) {
  const en = locale === "en"
  const index = en ? 1 : 0
  const mode = (online: boolean) =>
    online ? (en ? "Online" : "在线") : en ? "Local" : "本地"
  return (
    <div className={`model-tables ${detailed ? "model-tables-detailed" : ""}`}>
      <div className="model-table-scroll">
        <table className="model-table">
          <caption>
            {en ? "Speech recognition · ASR" : "语音识别 · ASR"}
          </caption>
          <thead>
            <tr>
              <th scope="col">{en ? "Model" : "模型"}</th>
              <th scope="col">{en ? "Runs" : "运行"}</th>
              <th scope="col">{en ? "Highlights" : "特点"}</th>
              {detailed && (
                <>
                  <th scope="col">{en ? "Native streaming" : "原生流式"}</th>
                  <th scope="col">{en ? "Hotwords" : "热词"}</th>
                </>
              )}
            </tr>
          </thead>
          <tbody>
            {asrModels.map((model) => (
              <tr key={model.name[1]}>
                <th scope="row">{model.name[index]}</th>
                <td>{mode(model.online)}</td>
                <td>{model.feature[index]}</td>
                {detailed && (
                  <>
                    <td>
                      {model.streaming
                        ? en
                          ? "Yes"
                          : "支持"
                        : en
                          ? "No"
                          : "不支持"}
                    </td>
                    <td>
                      {model.hotwords
                        ? en
                          ? "Yes"
                          : "支持"
                        : en
                          ? "No"
                          : "不支持"}
                    </td>
                  </>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="model-table-scroll">
        <table className="model-table">
          <caption>{en ? "Text polishing · LLM" : "文本润色 · LLM"}</caption>
          <thead>
            <tr>
              <th scope="col">
                {en ? "Service / runtime" : "服务 / 运行工具"}
              </th>
              <th scope="col">{en ? "Runs" : "运行"}</th>
            </tr>
          </thead>
          <tbody>
            {llmServices.map((service) => (
              <tr key={service.name}>
                <th scope="row">{service.name}</th>
                <td>{mode(service.online)}</td>
              </tr>
            ))}
            {detailed && (
              <tr>
                <th scope="row">
                  {en
                    ? "Custom OpenAI-compatible service"
                    : "自定义 OpenAI 兼容服务"}
                </th>
                <td>{en ? "Online / Local" : "在线 / 本地"}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  )
}

export function SupportedModels({ locale }: { locale: Locale }) {
  const en = locale === "en"
  return (
    <section
      className="wrap section-space supported-models"
      aria-labelledby="models-title"
    >
      <h2 id="models-title" className="reveal text-head">
        {en ? "Supported models" : "支持模型"}
      </h2>
      <p className="mt-6 max-w-3xl text-muted-foreground">
        {en
          ? "Bring your own API key for online models. Keep audio and text on your device with local recognition and polishing."
          : "在线模型，填写自己申请的 API Key；本地识别与润色，让音频和文本留在设备上。"}
      </p>
      <ModelTables locale={locale} />
      <Link className="model-guide mt-7" href={`/${locale}/docs/compatibility`}>
        {en ? "View compatibility list" : "查看完整兼容清单"} ↗
      </Link>
    </section>
  )
}
