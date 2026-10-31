import { AlertCircle, Check, ChevronRight, Loader2, Maximize2, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { normalizeLanguage, translate, type UiLanguage } from "@/i18n";
import {
  cancelPicker,
  getPickerState,
  onPickerClosing,
  onPickerState,
  type PickerOptionView,
  type PickerView,
  reportPickerLayout,
  selectPickerOption,
} from "./bridge";

function useWindowViewport(): { width: number; height: number } {
  const [viewport, setViewport] = useState(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));
  useEffect(() => {
    const onResize = () => setViewport({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return viewport;
}

/** Column ceiling for a measured viewport width (legacy fallback tiers). */
function columnsForWidth(width: number, count: number): number {
  const max = width < 520 ? 1 : width < 780 ? 2 : 4;
  return Math.max(1, Math.min(count || 1, max));
}

function applyTheme(theme: string) {
  const resolved =
    theme === "system"
      ? window.matchMedia("(prefers-color-scheme: light)").matches
        ? "light"
        : "dark"
      : theme;
  document.documentElement.dataset.theme = resolved;
}

/**
 * Language of the current picker session.
 *
 * The panel is a separate window that receives its language with the session
 * payload (the backend resolves the preference), so translations are read from
 * this module value instead of a React context. Every state update carries it,
 * and the render that consumes it happens right after.
 */
let currentLanguage: UiLanguage = "zh-CN";

function t(zh: string, vars?: Record<string, string | number | null | undefined>): string {
  return translate(currentLanguage, zh, vars);
}

function statusLabel(option: PickerOptionView): string {
  if (option.status === "loading") return t("处理中…");
  if (option.status === "failed") return t("处理失败");
  return option.kind === "original" ? t("识别原文") : t("已生成");
}

export function PickerApp() {
  const [view, setView] = useState<PickerView | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  // The window is reused across sessions and hidden natively, so the exit
  // animation needs an explicit signal: `closing` is set when the backend says it
  // is about to hide the window and cleared by the next session's first state.
  const [closing, setClosing] = useState(false);
  const sessionIdRef = useRef<number | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const { width, height } = useWindowViewport();

  useEffect(() => {
    let mounted = true;
    getPickerState()
      .then((initial) => {
        if (!mounted || !initial) return;
        sessionIdRef.current = initial.sessionId;
        currentLanguage = normalizeLanguage(initial.language);
        setView(initial);
      })
      .catch(() => {});
    const unlisten = onPickerState((next) => {
      if (!mounted) return;
      if (next.sessionId !== sessionIdRef.current) {
        // A new session owns the window again: replay the entrance animation.
        sessionIdRef.current = next.sessionId;
        setClosing(false);
      }
      currentLanguage = normalizeLanguage(next.language);
      setView(next);
    });
    const unlistenClosing = onPickerClosing((signal) => {
      if (!mounted) return;
      // `closing: false` is a failed-delivery re-arm on the same session:
      // return the panel to open instead of fading out invisibly.
      setClosing(signal.closing ?? true);
    });
    return () => {
      mounted = false;
      unlisten();
      unlistenClosing();
    };
  }, []);

  useEffect(() => {
    applyTheme(view?.theme ?? "system");
  }, [view?.theme]);

  // A new session starts fresh: collapse previews, re-enable selection, and
  // reset the scroll offset. The picker window is reused between sessions, so a
  // scroll position left by the previous session would otherwise open the new
  // one mid-card, clipping the first row's title at the header edge.
  // biome-ignore lint/correctness/useExhaustiveDependencies: session id is the reset trigger
  useEffect(() => {
    setExpanded(null);
    setSubmitting(false);
    scrollRef.current?.scrollTo({ top: 0 });
  }, [view?.sessionId]);

  useEffect(() => {
    if (!view?.delivering) setSubmitting(false);
  }, [view]);

  const options = view?.options ?? [];
  const columns = useMemo(() => {
    // The native layout owns the column policy: it is computed from the same
    // work area that sized the panel, so it never depends on `window.innerWidth`
    // lagging a cross-monitor resize of the reused window.
    //
    // It is still only a *request*: if this viewport measures narrower than
    // native expected (a constrained frame, a mid-transition resize), applying
    // it verbatim would clip the cards. So cap it by the measured width, and
    // let a viewport that widens again restore it — the grid re-renders on
    // every resize event.
    const fit = columnsForWidth(width, options.length);
    if (view?.columns && view.columns > 0) return Math.min(view.columns, fit);
    return fit;
  }, [view?.columns, width, options.length]);

  const desiredColumns = view?.columns && view.columns > 0 ? view.columns : columns;

  // One diagnostic per settled layout: what the page actually measures against
  // what native asked for. Divergence here is the "wrong size on the outer
  // monitor" symptom seen from the renderer side.
  useEffect(() => {
    if (!view) return;
    reportPickerLayout({
      sessionId: view.sessionId,
      viewportWidth: width,
      viewportHeight: height,
      scaleFactor: window.devicePixelRatio,
      desiredColumns,
      columns,
    });
  }, [view, width, height, desiredColumns, columns]);

  const select = useCallback(
    async (option: PickerOptionView) => {
      if (!view || view.delivering || submitting) return;
      if (option.status !== "ready" || !option.text) return;
      setSubmitting(true);
      setExpanded(null);
      // Same frame as the click: start the existing pop-out immediately. The
      // backend keeps the window alive for the animation while the clipboard
      // and paste run, and re-opens (closing: false) if delivery fails.
      setClosing(true);
      try {
        await selectPickerOption(view.sessionId, option.id);
      } catch {
        setSubmitting(false);
        setClosing(false);
      }
    },
    [submitting, view],
  );

  const dismiss = useCallback(async () => {
    if (!view) return;
    // The Esc control begins the same exit animation instantly; nothing is
    // pasted and the native hide lands when the animation ends.
    setClosing(true);
    try {
      await cancelPicker(view.sessionId);
    } catch {
      /* ESC is the authoritative path; a failed click-close is harmless */
      setClosing(false);
    }
  }, [view]);

  if (!view) {
    return (
      <div className="picker-root flex h-screen w-screen items-center justify-center p-2">
        <div className="picker-surface flex items-center gap-2 rounded-2xl border px-4 py-3 text-sm text-text-dim backdrop-blur-2xl">
          <Loader2 className="size-4 animate-spin" />
          {t("等待转写…")}
        </div>
      </div>
    );
  }

  const busy = view.delivering || submitting;
  const expandedOption = options.find((option) => option.id === expanded) ?? null;

  return (
    <div className="picker-root relative h-screen w-screen p-2">
      {/* Keyed by session so every session replays the entrance animation (and
          remounts the card list at the top). */}
      <section
        key={view.sessionId}
        data-picker-state={closing ? "closing" : "open"}
        className="picker-panel picker-surface flex h-full w-full flex-col overflow-hidden rounded-[20px] border bg-surface-card backdrop-blur-xl"
      >
        {/* The header is a flex item in its own right: `shrink-0` keeps it at its
            natural height so resizing can never squash it, the opaque background
            hides cards scrolling behind the scroll container's top edge, and `z-10`
            keeps the card hover lift from painting over it. */}
        <header className="relative z-10 flex shrink-0 items-center justify-between gap-3 border-b border-border/50 bg-surface-card px-4 pt-3 pb-2.5">
          <div className="min-w-0">
            <h1 className="truncate text-[13px] font-semibold text-text">
              {t("选择要粘贴的版本")}
            </h1>
            <p className="truncate text-[11px] text-text-muted">
              {view.targetApp
                ? t("粘贴到 {app}", { app: view.targetApp })
                : t("点击卡片或按对应快捷键插入")}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {view.allCompleted && (
              <span className="flex items-center gap-1 text-[11px] text-text-muted">
                <Check className="size-3.5" />
                {t("全部完成")}
              </span>
            )}
            <button
              type="button"
              onClick={dismiss}
              className="rounded-lg border border-border/70 px-2 py-1 text-[11px] text-text-dim transition hover:border-border-hover hover:text-text"
            >
              {t("Esc 取消")}
            </button>
          </div>
        </header>

        {view.error && (
          <div className="mx-4 mt-2 mb-2 flex shrink-0 items-start gap-2 rounded-xl border border-error/40 bg-error/10 px-3 py-2 text-[11.5px] text-error">
            <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">{view.error}</span>
          </div>
        )}

        {/* Scroll region: starts *below* the header, so the first card row and any
            row scrolled into view are clipped by this container's top edge, never
            drawn under the header. It owns the space between header and footer and
            scrolls on its own, so a long transcript never pushes the footer out of
            the window. */}
        <div
          ref={scrollRef}
          data-picker-scroll=""
          className="relative mt-1.5 min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pt-1 pb-2"
        >
          <div
            className={`grid h-full gap-2 ${options.length <= 2 ? "mx-auto max-w-[792px]" : ""}`}
            style={{
              gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              gridAutoRows: "minmax(148px, 1fr)",
            }}
          >
            {options.map((option) => (
              <PickerCard
                key={option.id}
                option={option}
                busy={busy}
                onSelect={() => select(option)}
                onExpand={() => setExpanded(option.id)}
              />
            ))}
          </div>
        </div>

        <footer className="flex shrink-0 items-center justify-between gap-3 border-t border-border/50 px-4 py-2 text-[10.5px] text-text-muted">
          <span className="truncate">{t("原文始终可用；模板独立处理，不会串联。")}</span>
          <span className="shrink-0">
            {busy ? (
              <span className="flex items-center gap-1.5 text-accent">
                <Loader2 className="size-3 animate-spin" />
                {t("正在插入…")}
              </span>
            ) : (
              t("Esc 不粘贴任何内容")
            )}
          </span>
        </footer>
      </section>

      {expandedOption && (
        <div className="absolute inset-0 flex items-center justify-center bg-black/45 p-3 backdrop-blur-sm">
          <div className="flex h-full w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-surface-card">
            <div className="flex items-center justify-between gap-3 border-b border-border/60 px-4 py-2.5">
              <div className="flex min-w-0 items-center gap-2">
                <h2 className="truncate text-[13px] font-semibold text-text">
                  {expandedOption.title}
                </h2>
                {expandedOption.hotkey && <Kbd>{expandedOption.hotkey}</Kbd>}
              </div>
              <button
                type="button"
                onClick={() => setExpanded(null)}
                className="rounded-lg p-1 text-text-dim transition hover:bg-fill-hover hover:text-text"
                aria-label={t("关闭预览")}
              >
                <X className="size-4" />
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
              <p className="whitespace-pre-wrap break-words text-[12.5px] leading-relaxed text-text">
                {expandedOption.text}
              </p>
            </div>
            <div className="flex items-center justify-between gap-2 border-t border-border/60 px-3 py-2">
              <span className="text-[10.5px] text-text-muted">{t("预览不会粘贴")}</span>
              <button
                type="button"
                disabled={busy || expandedOption.status !== "ready"}
                onClick={() => select(expandedOption)}
                className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
              >
                {t("选择此版本")}
                <ChevronRight className="size-3.5" />
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="shrink-0 rounded-md border border-border/80 bg-fill-subtle px-1.5 py-0.5 text-[10.5px] font-medium text-text-dim">
      {children}
    </kbd>
  );
}

function PickerCard({
  option,
  busy,
  onSelect,
  onExpand,
}: {
  option: PickerOptionView;
  busy: boolean;
  onSelect: () => void;
  onExpand: () => void;
}) {
  const ready = option.status === "ready" && !!option.text;
  const selectable = ready && !busy;

  return (
    <div className="relative flex min-h-0">
      <button
        type="button"
        disabled={!selectable}
        aria-label={`${option.title}${option.hotkey ? `（${option.hotkey}）` : ""}`}
        onClick={onSelect}
        className={`flex h-full w-full min-w-0 flex-col rounded-xl border p-3 text-left transition duration-100 ${
          option.kind === "original" ? "border-accent/45 bg-accent-soft/30" : "border-border/70"
        } ${
          // 1px accent ring only — the panel's design language forbids drop
          // shadows, and the 2px hover lift needs the scroll region's pt-1
          // room so the card's own top border never clips at the edge.
          selectable
            ? "cursor-pointer hover:-translate-y-0.5 hover:border-accent hover:bg-accent-soft/40 hover:shadow-[0_0_0_1px_var(--color-accent-dim)]"
            : "cursor-default opacity-80"
        } ${busy ? "pointer-events-none opacity-60" : ""}`}
      >
        <div className="flex items-start justify-between gap-2 pr-7">
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-[13px] font-semibold text-text">{option.title}</span>
              {option.kind === "original" && (
                <span className="shrink-0 rounded-full bg-accent/15 px-1.5 py-0.5 text-[9.5px] font-medium text-accent">
                  {t("原文")}
                </span>
              )}
            </div>
            <p className="mt-0.5 truncate text-[10.5px] text-text-muted">{statusLabel(option)}</p>
          </div>
          {option.hotkey && <Kbd>{option.hotkey}</Kbd>}
        </div>

        <div className="mt-2 min-h-0 flex-1 overflow-y-auto pr-0.5">
          {option.status === "loading" && <LoadingPreview />}
          {option.status === "failed" && (
            <div className="flex items-start gap-1.5 text-[11.5px] leading-relaxed text-error">
              <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
              <span className="break-words">{option.error || t("处理失败")}</span>
            </div>
          )}
          {option.status === "ready" && (
            <p className="whitespace-pre-wrap break-words text-[12.5px] leading-relaxed text-text-dim">
              {option.text || t("（空）")}
            </p>
          )}
        </div>

        {ready && (
          <div className="mt-2 flex items-center justify-end text-[10px] text-text-muted">
            <span>{t("点击插入")}</span>
            <ChevronRight className="size-3" />
          </div>
        )}
      </button>

      {ready && (
        <button
          type="button"
          onClick={onExpand}
          className="absolute top-2 right-2 rounded-md p-1 text-text-muted transition hover:bg-fill-hover hover:text-text"
          aria-label={t("展开 {title}", { title: option.title })}
        >
          <Maximize2 className="size-3.5" />
        </button>
      )}
    </div>
  );
}

function LoadingPreview() {
  return (
    <div className="space-y-1.5">
      <div className="h-2.5 w-full animate-pulse rounded bg-fill-subtle" />
      <div className="h-2.5 w-11/12 animate-pulse rounded bg-fill-subtle" />
      <div className="h-2.5 w-2/3 animate-pulse rounded bg-fill-subtle" />
      <div className="flex items-center gap-1.5 pt-1 text-[10.5px] text-text-muted">
        <Loader2 className="size-3 animate-spin" />
        {t("处理中…")}
      </div>
    </div>
  );
}
