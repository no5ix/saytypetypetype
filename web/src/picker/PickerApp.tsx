import { AlertCircle, Check, ChevronRight, Loader2, Maximize2, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  cancelPicker,
  getPickerState,
  onPickerClosing,
  onPickerState,
  type PickerOptionView,
  type PickerView,
  selectPickerOption,
} from "./bridge";

function useWindowWidth(): number {
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return width;
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

function statusLabel(option: PickerOptionView): string {
  if (option.status === "loading") return "处理中…";
  if (option.status === "failed") return "处理失败";
  return option.kind === "original" ? "识别原文" : "已生成";
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
  const width = useWindowWidth();

  useEffect(() => {
    let mounted = true;
    getPickerState()
      .then((initial) => {
        if (!mounted || !initial) return;
        sessionIdRef.current = initial.sessionId;
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
      setView(next);
    });
    const unlistenClosing = onPickerClosing(() => {
      if (mounted) setClosing(true);
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

  // A new session starts fresh: collapse previews, re-enable selection, and put
  // the card list back at the top. The window is reused between sessions, so a
  // scroll offset left by the previous one would otherwise open the new session
  // mid-card and make the first row look cut off under the header.
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
    if (width < 520) return 1;
    const max = width < 780 ? 2 : 4;
    return Math.max(1, Math.min(options.length || 1, max));
  }, [width, options.length]);

  const select = useCallback(
    async (option: PickerOptionView) => {
      if (!view || view.delivering || submitting) return;
      if (option.status !== "ready" || !option.text) return;
      setSubmitting(true);
      setExpanded(null);
      try {
        await selectPickerOption(view.sessionId, option.id);
      } catch {
        setSubmitting(false);
      }
    },
    [submitting, view],
  );

  const dismiss = useCallback(async () => {
    if (!view) return;
    try {
      await cancelPicker(view.sessionId);
    } catch {
      /* ESC is the authoritative path; a failed click-close is harmless */
    }
  }, [view]);

  if (!view) {
    return (
      <div className="picker-root flex h-screen w-screen items-center justify-center p-2">
        <div className="picker-surface flex items-center gap-2 rounded-2xl border px-4 py-3 text-sm text-text-dim backdrop-blur-2xl">
          <Loader2 className="size-4 animate-spin" />
          等待转写…
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
        {/* Header and footer are flex items that own their space (`shrink-0`), so
            the cards always get exactly what is left between them and nothing is
            ever squashed or overlapped when the window is resized. */}
        <header className="flex shrink-0 items-center justify-between gap-3 px-4 pt-3 pb-2">
          <div className="min-w-0">
            <h1 className="truncate text-[13px] font-semibold text-text">选择要粘贴的版本</h1>
            <p className="truncate text-[11px] text-text-muted">
              {view.targetApp ? `粘贴到 ${view.targetApp}` : "点击卡片或按对应快捷键插入"}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {view.allCompleted && (
              <span className="flex items-center gap-1 text-[11px] text-text-muted">
                <Check className="size-3.5" />
                全部完成
              </span>
            )}
            <button
              type="button"
              onClick={dismiss}
              className="rounded-lg border border-border/70 px-2 py-1 text-[11px] text-text-dim transition hover:border-border-hover hover:text-text"
            >
              Esc 取消
            </button>
          </div>
        </header>

        {view.error && (
          <div className="mx-4 mb-2 flex shrink-0 items-start gap-2 rounded-xl border border-error/40 bg-error/10 px-3 py-2 text-[11.5px] text-error">
            <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">{view.error}</span>
          </div>
        )}

        {/* The cards own the space between header and footer and scroll on their
            own, so a long transcript never pushes the footer out of the window and
            never paints under the header. */}
        <div
          ref={scrollRef}
          data-picker-scroll=""
          className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pt-1.5 pb-2"
        >
          <div
            className={`grid h-full gap-2 ${options.length <= 2 ? "mx-auto max-w-[792px]" : ""}`}
            style={{
              gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              gridAutoRows: "minmax(156px, 1fr)",
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
          <span className="truncate">原文始终可用；模板独立处理，不会串联。</span>
          <span className="shrink-0">
            {busy ? (
              <span className="flex items-center gap-1.5 text-accent">
                <Loader2 className="size-3 animate-spin" />
                正在插入…
              </span>
            ) : (
              "Esc 不粘贴任何内容"
            )}
          </span>
        </footer>
      </section>

      {expandedOption && (
        <div className="absolute inset-0 flex items-center justify-center bg-black/45 p-3 backdrop-blur-sm">
          <div className="flex h-full w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-surface-card shadow-2xl">
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
                aria-label="关闭预览"
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
              <span className="text-[10.5px] text-text-muted">预览不会粘贴</span>
              <button
                type="button"
                disabled={busy || expandedOption.status !== "ready"}
                onClick={() => select(expandedOption)}
                className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
              >
                选择此版本
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
        className={`flex h-full w-full min-w-0 flex-col rounded-xl border p-3 text-left transition ${
          option.kind === "original" ? "border-accent/45 bg-accent-soft/30" : "border-border/70"
        } ${
          selectable
            ? "cursor-pointer hover:-translate-y-0.5 hover:border-border-hover hover:bg-fill-subtle/60"
            : "cursor-default opacity-80"
        } ${busy ? "pointer-events-none opacity-60" : ""}`}
      >
        <div className="flex items-start justify-between gap-2 pr-7">
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-[13px] font-semibold text-text">{option.title}</span>
              {option.kind === "original" && (
                <span className="shrink-0 rounded-full bg-accent/15 px-1.5 py-0.5 text-[9.5px] font-medium text-accent">
                  原文
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
              <span className="break-words">{option.error || "处理失败"}</span>
            </div>
          )}
          {option.status === "ready" && (
            <p className="whitespace-pre-wrap break-words text-[12.5px] leading-relaxed text-text-dim">
              {option.text || "（空）"}
            </p>
          )}
        </div>

        {ready && (
          <div className="mt-2 flex items-center justify-end text-[10px] text-text-muted">
            <span>点击插入</span>
            <ChevronRight className="size-3" />
          </div>
        )}
      </button>

      {ready && (
        <button
          type="button"
          onClick={onExpand}
          className="absolute top-2 right-2 rounded-md p-1 text-text-muted transition hover:bg-fill-hover hover:text-text"
          aria-label={`展开 ${option.title}`}
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
        处理中…
      </div>
    </div>
  );
}
