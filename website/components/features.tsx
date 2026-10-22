import Image from "next/image"
import Link from "next/link"
import type { Locale } from "@/lib/i18n"
import { SoundDemo, StreamDemo, LanguageDemo } from "./product-demos"

function ModelChoices({ locale }: { locale: Locale }) {
  const en = locale === "en"
  const groups = [
    {
      title: en ? "Online · Your API key" : "在线 · 自带 API Key",
      brands: [
        { name: "ElevenLabs", icon: "elevenlabs" },
        { name: "OpenAI", icon: "openai" },
      ],
    },
    {
      title: en ? "Local · On your device" : "本地 · 在设备上运行",
      brands: [
        { name: "Qwen", icon: "qwen" },
        { name: "Ollama", icon: "ollama" },
      ],
    },
  ]
  return (
    <article className="feature-card feature-wide">
      <div className="feature-copy">
        <h3 className="text-lead">
          {en ? "Your models. Your choice." : "模型，由你选择。"}
        </h3>
        <p className="mt-4">
          {en
            ? "Bring your own API key, or keep transcription and polishing on your device."
            : "在线自带 API Key，也可以选择本地识别与润色。"}
        </p>
      </div>
      <div className="feature-canvas model-canvas">
        <div className="model-groups">
          {groups.map((group) => (
            <div key={group.title} className="model-group">
              <p className="model-group-title">{group.title}</p>
              <div className="model-brands">
                {group.brands.map((brand) => (
                  <div key={brand.name} className="model-brand">
                    <Image
                      src={`/assets/brands/${brand.icon}.svg`}
                      width={40}
                      height={40}
                      alt=""
                      className="model-brand-icon"
                    />
                    <span>{brand.name}</span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
        <Link href={`/${locale}/docs/models`} className="model-guide">
          {en ? "Explore model settings" : "查看模型配置"} ↗
        </Link>
      </div>
    </article>
  )
}
function IllustratedFeature({
  locale,
  type,
}: {
  locale: Locale
  type: "templates" | "hotwords"
}) {
  const en = locale === "en"
  const template = type === "templates"
  const title = template
    ? en
      ? "A template for every task."
      : "不同表达，各有模板。"
    : en
      ? "Make your vocabulary count."
      : "你的词，记得更准。"
  const description = template
    ? en
      ? "Separate prompts and shortcuts, for the way you want to write."
      : "独立提示词、独立快捷键，让输出符合这一次的需要。"
    : en
      ? "Add names, products and specialist terms to your hotword lists."
      : "人名、产品名、专业术语，都可以加入热词库。"
  const alt = template
    ? en
      ? "Loose thoughts arranged into a letter, a checklist and a note"
      : "零散想法整理成邮件、清单和笔记的概念配图"
    : en
      ? "Personal vocabulary illustrated with VoicePaste, Ollama and macOS"
      : "以 VoicePaste、Ollama 和 macOS 展示自定义词汇的概念配图"
  return (
    <article className="feature-card feature-wide">
      <div className="feature-copy">
        <h3 className="text-lead">{title}</h3>
        <p className="mt-4">{description}</p>
      </div>
      <div className="feature-canvas illustration-canvas">
        <Image
          src={`/assets/illustrations/${type}.png`}
          width={1536}
          height={1024}
          alt={alt}
          sizes="(max-width: 767px) calc(100vw - 40px), (max-width: 1279px) 62vw, 760px"
          className="feature-illustration"
        />
      </div>
    </article>
  )
}
export function Features({ locale }: { locale: Locale }) {
  const en = locale === "en"
  return (
    <section
      id="features"
      className="section-space features-section"
      aria-labelledby="features-title"
    >
      <div className="wrap">
        <h2 id="features-title" className="reveal text-head">
          {en ? (
            <>
              Keep your train
              <br />
              of thought.
            </>
          ) : (
            <>
              思路不用停，
              <br />
              表达慢慢来。
            </>
          )}
        </h2>
        <div className="feature-grid">
          <ModelChoices locale={locale} />
          <SoundDemo locale={locale} />
          <StreamDemo locale={locale} />
          <IllustratedFeature locale={locale} type="templates" />
          <IllustratedFeature locale={locale} type="hotwords" />
          <LanguageDemo locale={locale} />
        </div>
      </div>
    </section>
  )
}
