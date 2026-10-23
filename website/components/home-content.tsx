import Link from "next/link"
import Image from "next/image"
import { FAQ } from "./faq"
import { Features } from "./features"
import { SupportedModels } from "./supported-models"
import type { Locale } from "@/lib/i18n"
import { DownloadButton } from "./download-button"
import { ScenarioDemo, AutoVideo, RevealObserver } from "./product-demos"
export function HomeContent({ locale }: { locale: Locale }) {
  return (
    <div className="marketing">
      <RevealObserver />
      <main id="main">
        <section
          className="hero wrap pt-12 text-center max-sm:pt-8"
          aria-labelledby="hero-title"
        >
          <h1
            id="hero-title"
            className="hero-heading reveal mx-auto max-w-3xl text-muted-foreground"
          >
            {locale === "en"
              ? "A free, lightweight, open-source desktop voice tool."
              : "免费、轻量、开源的桌面语音输入工具。"}
          </h1>
          <p className="hero-tagline reveal mt-5 text-display">
            {locale === "en" ? (
              <>
                Think freely.
                <br />
                Speak naturally.
              </>
            ) : (
              "慢慢想，随意说。"
            )}
          </p>
          <p className="hero-description reveal mx-auto mt-6 max-w-2xl">
            {locale === "en"
              ? "VoicePaste turns scattered thoughts into clear writing."
              : "VoicePaste 帮你把零散想法整理成清楚的文字。"}
          </p>
          <div className="reveal mt-8 flex flex-col items-center">
            <div className="flex flex-wrap gap-3">
              <DownloadButton platform="macos-arm64" secondary={false} />
              <DownloadButton platform="macos-x64" secondary={true} />
              <DownloadButton platform="windows-x64" secondary={true} />
            </div>
            <Link
              className="more-downloads mt-5 inline-block text-sm"
              href={`/${locale}/downloads#history-title`}
            >
              <span>
                {locale === "en" ? (
                  <>{"Previous releases →"}</>
                ) : (
                  <>{"历史版本 →"}</>
                )}
              </span>
            </Link>
          </div>
          <AutoVideo locale={locale} />
        </section>
        <ScenarioDemo locale={locale} />
        <SupportedModels locale={locale} />
        <Features locale={locale} />
        <section
          id="faq"
          className="wrap section-space grid gap-10 lg:grid-cols-[.7fr_1.3fr] lg:gap-16"
          aria-labelledby="faq-title"
        >
          <div>
            <h2 id="faq-title" className="reveal text-head">
              {locale === "en" ? (
                <>{"A few more answers."}</>
              ) : (
                <>{"你可能还想知道。"}</>
              )}
            </h2>
            <Link
              className="text-link mt-6 inline-block"
              href={`/${locale}/docs`}
            >
              <span>
                {locale === "en" ? (
                  <>{"Read the docs ↗"}</>
                ) : (
                  <>{"查看配置文档 ↗"}</>
                )}
              </span>
            </Link>
          </div>
          <FAQ locale={locale} />
        </section>
        <section
          id="download"
          className="section-space bg-muted py-20 text-foreground"
        >
          <div className="wrap grid items-center gap-10 md:grid-cols-[1.3fr_.7fr]">
            <div>
              <h2 className="reveal text-display">
                {locale === "en" ? (
                  <>
                    {"Think it."}
                    <br />
                    {"Say it."}
                  </>
                ) : (
                  <>
                    {"下一句话，"}
                    <br />
                    {"试着说出来。"}
                  </>
                )}
              </h2>
              <div className="reveal mt-8">
                <div className="flex flex-wrap gap-3">
                  <DownloadButton platform="macos-arm64" secondary={false} />
                  <DownloadButton platform="macos-x64" secondary={true} />
                  <DownloadButton platform="windows-x64" secondary={true} />
                </div>
                <Link
                  className="more-downloads mt-5 inline-block text-sm"
                  href={`/${locale}/downloads#history-title`}
                >
                  <span>
                    {locale === "en" ? (
                      <>{"Previous releases →"}</>
                    ) : (
                      <>{"历史版本 →"}</>
                    )}
                  </span>
                </Link>
              </div>
            </div>
            <Image
              sizes="(max-width: 768px) 100vw, 600px"
              className="closing-mark reveal w-full max-w-xs justify-self-end max-sm:max-w-48"
              src="/assets/mark.png"
              width={1024}
              height={1024}
              alt=""
            />
          </div>
        </section>
      </main>
    </div>
  )
}
