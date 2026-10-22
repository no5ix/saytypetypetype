"use client"
import { useEffect, useRef, useState, type RefObject } from "react"
import { Volume2, Check } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { Locale } from "@/lib/i18n"
import { examples } from "@/lib/scenarios"

// One clock per visible demo; all timers and observers are released on navigation.
function useDemoClock(ref: RefObject<HTMLElement | null>, duration: number) {
  const [clock, setClock] = useState({ time: 0, running: false, reduced: true })
  const [restart, setRestart] = useState(0)
  useEffect(() => {
    const node = ref.current
    if (!node) return
    const motion = matchMedia("(prefers-reduced-motion: reduce)")
    let visible = false
    let timer: ReturnType<typeof setInterval> | undefined
    const stop = () => {
      clearInterval(timer)
      timer = undefined
    }
    const sync = () => {
      stop()
      const running = visible && !document.hidden && !motion.matches
      setClock({ time: 0, running, reduced: motion.matches })
      if (!running) return
      const start = performance.now()
      timer = setInterval(
        () =>
          setClock({
            time: (performance.now() - start) % duration,
            running: true,
            reduced: false,
          }),
        70
      )
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        visible = entry.isIntersecting
        sync()
      },
      { threshold: 0.25 }
    )
    observer.observe(node)
    document.addEventListener("visibilitychange", sync)
    motion.addEventListener("change", sync)
    return () => {
      stop()
      observer.disconnect()
      document.removeEventListener("visibilitychange", sync)
      motion.removeEventListener("change", sync)
    }
  }, [duration, restart, ref])
  return { ...clock, restart: () => setRestart((value) => value + 1) }
}
function typed(text: string, elapsed: number, duration = 2400) {
  return text.slice(
    0,
    Math.max(
      0,
      Math.min(text.length, Math.ceil((text.length * elapsed) / duration))
    )
  )
}
function Waveform() {
  return (
    <span className="waveform" aria-hidden="true">
      <i />
      <i />
      <i />
      <i />
    </span>
  )
}
function Pill({
  children,
  phase = "recording",
}: {
  children: string
  phase?: "recording" | "processing" | "complete"
}) {
  const confirmed =
    phase === "recording" ? Math.max(0, children.length - 6) : children.length
  return (
    <div
      className={`voice-pill voice-pill-${phase}`}
      data-ui
      data-phase={phase}
    >
      {phase === "recording" && <Waveform />}
      {phase === "complete" && (
        <Check className="size-4 shrink-0" aria-hidden="true" />
      )}
      <span className="pill-transcript">
        <span>{children.slice(0, confirmed)}</span>
        <span className="opacity-55">{children.slice(confirmed)}</span>
      </span>
    </div>
  )
}
const modes = ["input", "translate", "polish"] as const
export function ScenarioDemo({ locale }: { locale: Locale }) {
  const ref = useRef<HTMLDivElement>(null)
  const clock = useDemoClock(ref, 24000)
  const [base, setBase] = useState(0)
  const index = (base + (clock.running ? Math.floor(clock.time / 8000) : 0)) % 3
  const mode = modes[index]
  const data = examples[mode][locale]
  const elapsed = clock.time % 8000
  const phase =
    clock.reduced || !clock.running
      ? "complete"
      : elapsed < 4000
        ? "recording"
        : elapsed < 4800
          ? "processing"
          : "complete"
  const en = locale === "en"
  const output =
    phase === "complete"
      ? clock.reduced || !clock.running
        ? data[5]
        : typed(data[5], elapsed - 4800, 1500)
      : ""
  const transcript =
    phase === "recording"
      ? typed(data[4], elapsed, 3200)
      : phase === "processing"
        ? en
          ? "Preparing your text…"
          : "整理中…"
        : en
          ? "Text delivered"
          : "输入完成"
  return (
    <section
      ref={ref}
      id="scenarios"
      className="wrap section-space"
      aria-labelledby="scenarios-title"
    >
      <div className="mb-10 flex flex-wrap items-end justify-between gap-8">
        <h2 id="scenarios-title" className="reveal text-head">
          {en ? (
            <>
              One thought.
              <br />
              Many ways to write.
            </>
          ) : (
            <>
              一句口述，
              <br />
              多种表达。
            </>
          )}
        </h2>
        <div
          className="flex flex-wrap gap-2"
          role="tablist"
          aria-label={en ? "Writing mode" : "表达方式"}
        >
          {modes.map((item, i) => (
            <Button
              key={item}
              id={`tab-${item}`}
              role="tab"
              variant={index === i ? "default" : "ghost"}
              className="min-h-11 px-4"
              aria-selected={index === i}
              aria-controls="scenario-panel"
              tabIndex={index === i ? 0 : -1}
              onClick={() => {
                setBase(i)
                clock.restart()
              }}
              onKeyDown={(event) => {
                let next: number
                if (event.key === "ArrowRight") next = (i + 1) % 3
                else if (event.key === "ArrowLeft") next = (i + 2) % 3
                else if (event.key === "Home") next = 0
                else if (event.key === "End") next = 2
                else return
                event.preventDefault()
                setBase(next)
                clock.restart()
                document.getElementById(`tab-${modes[next]}`)?.focus()
              }}
            >
              {
                (en
                  ? ["Voice input", "Translate", "Polish"]
                  : ["语音输入", "翻译", "文本润色"])[i]
              }
            </Button>
          ))}
        </div>
      </div>
      <div
        className="grid items-center gap-10 lg:grid-cols-[.7fr_1.3fr] lg:gap-16"
        id="scenario-panel"
        role="tabpanel"
        aria-labelledby={`tab-${mode}`}
      >
        <div>
          <h3 className="text-lead">{data[2]}</h3>
          <p className="mt-5 max-w-sm">{data[3]}</p>
        </div>
        <div
          className={`demo-desktop relative flex min-h-[430px] flex-col items-center justify-center gap-7 overflow-hidden p-8 max-sm:min-h-[400px] max-sm:px-5 ${phase === "recording" && clock.running ? "is-running" : ""}`}
          data-ui
          data-phase={phase}
        >
          <div className="writing-window w-full max-w-lg">
            <div className="window-top flex items-center justify-between gap-4">
              <span className="window-dots" aria-hidden="true">
                <i />
                <i />
                <i />
              </span>
              <span>{data[0]}</span>
              <span aria-hidden="true">···</span>
            </div>
            <div className="min-h-[170px] px-7 py-6">
              <p className="mb-4 font-medium">{data[1]}</p>
              <p className="leading-relaxed whitespace-pre-line">{output}</p>
            </div>
          </div>
          <Pill phase={phase}>{transcript}</Pill>
        </div>
      </div>
    </section>
  )
}
export function SoundDemo({ locale }: { locale: Locale }) {
  const ref = useRef<HTMLDivElement>(null)
  const clock = useDemoClock(ref, 7000)
  const [enabled, setEnabled] = useState(false)
  const en = locale === "en"
  const phase = clock.time < 3500 ? "start" : "end"
  const audio = useRef<{
    start: HTMLAudioElement
    end: HTMLAudioElement
  } | null>(null)
  useEffect(() => {
    audio.current = {
      start: new Audio("/assets/start.mp3"),
      end: new Audio("/assets/end.mp3"),
    }
    return () => {
      Object.values(audio.current ?? {}).forEach((cue) => {
        cue.pause()
        cue.currentTime = 0
      })
      audio.current = null
    }
  }, [])
  useEffect(() => {
    const cues = audio.current
    if (!cues) return
    Object.values(cues).forEach((cue) => {
      cue.pause()
      cue.currentTime = 0
    })
    if (enabled && clock.running)
      cues[phase].play().catch(() => setEnabled(false))
    return () =>
      Object.values(cues).forEach((cue) => {
        cue.pause()
      })
  }, [enabled, phase, clock.running])
  return (
    <article
      ref={ref}
      className={`feature-card ${clock.running ? "is-running" : ""}`}
      id="sound-feature"
    >
      <h3 className="text-lead">
        {en ? "Hear when it happens." : "每次输入，都有回应。"}
      </h3>
      <p className="mt-4">
        {en
          ? "A clear sound when recording starts and your text is ready."
          : "开始录音、完成输入，用声音给你明确回应。"}
      </p>
      <div className="sound-demo feature-canvas sound-canvas" data-ui>
        <div className="flex items-center justify-between gap-4">
          <span className="sound-phase">
            {phase === "start"
              ? en
                ? "Recording started"
                : "开始录音"
              : en
                ? "Text delivered"
                : "输入完成"}
          </span>
          <Button
            variant="ghost"
            className="min-h-11"
            aria-pressed={enabled}
            onClick={() => setEnabled(!enabled)}
          >
            <Volume2 />
            {enabled
              ? en
                ? "Mute sound"
                : "关闭声音"
              : en
                ? "Enable sound"
                : "开启声音"}
          </Button>
        </div>
        <div
          className="sound-timeline mt-6 flex items-center gap-2"
          aria-hidden="true"
        >
          {Array.from({ length: 24 }, (_, i) => (
            <i key={i} />
          ))}
        </div>
      </div>
    </article>
  )
}
export function StreamDemo({ locale }: { locale: Locale }) {
  const ref = useRef<HTMLDivElement>(null)
  const clock = useDemoClock(ref, 6500)
  const en = locale === "en"
  const text = en
    ? "Give each thought a little room to become clear."
    : "把脑海里的想法，一点点说清楚。"
  return (
    <article ref={ref} className="feature-card" id="stream-feature">
      <h3 className="text-lead">
        {en ? "See it as you say it." : "说到哪里，看到哪里。"}
      </h3>
      <p className="mt-4">
        {en
          ? "Follow your words as they appear. Stop whenever you need to rethink."
          : "转写逐步呈现，随时确认、及时停止。"}
      </p>
      <div className="stream-demo feature-canvas stream-canvas" data-ui>
        <p className="stream-line">
          {clock.running ? typed(text, clock.time, 3200) : text}
        </p>
      </div>
    </article>
  )
}
export function AutoVideo({ locale }: { locale: Locale }) {
  const ref = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    const video = ref.current
    if (!video) return
    let visible = false
    const motion = matchMedia("(prefers-reduced-motion: reduce)")
    const sync = () => {
      if (!visible || document.hidden || motion.matches) video.pause()
      else void video.play().catch(() => {})
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        visible = entry.isIntersecting
        sync()
      },
      { threshold: 0.25 }
    )
    observer.observe(video)
    document.addEventListener("visibilitychange", sync)
    motion.addEventListener("change", sync)
    return () => {
      observer.disconnect()
      video.pause()
      document.removeEventListener("visibilitychange", sync)
      motion.removeEventListener("change", sync)
    }
  }, [])
  return (
    <video
      ref={ref}
      className="hero-video reveal mx-auto mt-12 block h-auto w-full max-w-5xl"
      width={1116}
      height={648}
      poster="/assets/demo-poster.jpg"
      preload="metadata"
      muted
      loop
      playsInline
      aria-label={
        locale === "en"
          ? "VoicePaste dictation, translation and polishing demo"
          : "VoicePaste 语音输入、翻译与润色演示"
      }
    >
      <source src="/assets/demo.mp4" type="video/mp4" />
    </video>
  )
}
export function RevealObserver() {
  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries)
          if (entry.isIntersecting) {
            entry.target.classList.add("arrived")
            observer.unobserve(entry.target)
          }
      },
      { threshold: 0.08 }
    )
    document
      .querySelectorAll(".reveal")
      .forEach((node) => observer.observe(node))
    return () => observer.disconnect()
  }, [])
  return null
}

export function LanguageDemo({ locale }: { locale: Locale }) {
  const ref = useRef<HTMLDivElement>(null)
  const clock = useDemoClock(ref, 8500)
  const en = locale === "en"
  const text = "这个版本的 API，我想再 review 一下。"
  return (
    <article ref={ref} className="feature-card">
      <div className="feature-copy">
        <h3 className="text-lead">
          {en ? "Speak your own way." : "按习惯说。"}
        </h3>
        <p className="mt-4">
          {en
            ? "Mix Chinese and English, or choose a model for the languages you use."
            : "中英混说、多语言识别，选择适合你的模型。"}
        </p>
      </div>
      <div className="feature-canvas language-canvas" data-ui>
        <div className="language-fragments" aria-hidden="true">
          <span className={clock.time < 4000 ? "language-active" : ""}>
            你好
          </span>
          <span className={clock.time >= 4000 ? "language-active" : ""}>
            Hello
          </span>
        </div>
        <p className="language-output" lang="zh-CN">
          {clock.running ? typed(text, clock.time, 3500) : text}
        </p>
      </div>
    </article>
  )
}
