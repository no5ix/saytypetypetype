import { Check, Info } from "lucide-react";
import { type CSSProperties, useCallback, useEffect, useState } from "react";
import { normalizeLanguage, translate, type UiLanguage } from "@/i18n";
import {
  getOverlayLayoutMetrics,
  getUiLanguage,
  type OverlayLayoutMetrics,
  onLanguageChanged,
} from "./bridge";
import type { OverlayState } from "./types";
import { useOverlayLayout } from "./useOverlayLayout";
import { useOverlayState } from "./useOverlayState";
import { useOverlayVisibility } from "./useOverlayVisibility";
import { Waveform } from "./Waveform";

// Fallback layout metrics used before the backend fetch resolves. Mirrors
// src-tauri/src/overlay/shared.rs (the single source of truth); the overlay is
// normally hidden during this brief window.
const FALLBACK_METRICS: OverlayLayoutMetrics = {
  pad_left: 16,
  pad_right: 16,
  indicator_w: 16,
  gap: 12,
  pill_h_single: 40,
  single_line_limit: 520,
  multi_line_width: 520,
  min_pill_w: 48,
  text_slack: 10,
  max_text_width: 520,
  max_text_lines: 3,
  wave_area_w: 22,
  wave_bar_w: 4,
  wave_bar_gap: 2,
  wave_corner_radius: 2,
  wave_min_h: 4,
  wave_max_h: 22,
  wave_durations: [0.85, 0.7, 1.0, 0.8],
  wave_peaks: [0.65, 1.0, 0.85, 0.55],
  wave_settle_seconds: 0.18,
};

/**
 * Language of the recording panel.
 *
 * The backend resolves the preference (system / zh-CN / en) and owns every
 * string it sends; these are the few placeholders the renderer derives itself,
 * kept aligned with `overlay::shared::visible_hint`. The value is module level
 * because the panel updates it from an event rather than from props.
 */
let currentLanguage: UiLanguage = "zh-CN";

function t(zh: string): string {
  return translate(currentLanguage, zh);
}

// Visible hint text — aligned 1:1 with src-tauri/src/overlay/shared.rs::visible_hint.
function visibleHintText(state: OverlayState): string {
  if (state.hintVariant === "success") return state.hintText;
  if (state.appState === "connecting") return t("准备中");
  if (state.appState === "finishing") {
    if (state.hintVariant === "retry") {
      if (!state.finalText && !state.partialText) return t("重试中");
      return "";
    }
    if (state.hintVariant === "progress") return t("润色中");
    if (!state.hintText) return t("识别中");
  }
  return state.hintText;
}

export function OverlayApp() {
  const { state, waveActive } = useOverlayState();
  const [lastShortcuts, setLastShortcuts] = useState(state.shortcuts);
  useEffect(() => {
    if (state.shortcuts.length > 0) setLastShortcuts(state.shortcuts);
  }, [state.shortcuts]);
  const displayedShortcuts = state.shortcuts.length > 0 ? state.shortcuts : lastShortcuts;

  const [, forceLanguage] = useState(0);
  const [metrics, setMetrics] = useState<OverlayLayoutMetrics>(FALLBACK_METRICS);

  // Language arrives from the backend (it owns the system-locale detection) and
  // is re-read whenever the resolved value changes.
  useEffect(() => {
    getUiLanguage()
      .then((language) => {
        currentLanguage = normalizeLanguage(language);
        forceLanguage((n) => n + 1);
      })
      .catch(() => {});
    return onLanguageChanged((language) => {
      currentLanguage = normalizeLanguage(language);
      forceLanguage((n) => n + 1);
    });
  }, []);
  const refetch = useCallback(() => {
    getOverlayLayoutMetrics()
      .then(setMetrics)
      .catch(() => {});
  }, []);
  useEffect(() => {
    refetch();
    // The backend sizes the overlay per session (each new dictation targets
    // the app's display); refetch the caps so the pill can use the new shell.
    window.addEventListener("resize", refetch);
    return () => window.removeEventListener("resize", refetch);
  }, [refetch]);

  const visibleHint = visibleHintText(state);
  const hasHint = Boolean(visibleHint);
  const isError = state.hintLevel === "error";
  const success = state.hintVariant === "success";
  const showWave = state.appState === "recording" && state.hintLevel !== "warn" && !isError;
  const showIcon = isError || success;
  const hasText = Boolean(state.finalText || state.partialText);
  const centerText = hasHint && !showWave && !showIcon;
  const shouldPulse = hasHint && !isError && !success && state.appState !== "idle";

  useEffect(() => {
    document.documentElement.dataset.overlayTheme = state.theme;
  }, [state.theme]);

  const { measureRef, pillRef, transcriptRef, wrap } = useOverlayLayout({
    finalText: state.finalText,
    partialText: state.partialText,
    visibleHintText: visibleHint,
    showIcon,
    appState: state.appState,
    hintLevel: state.hintLevel,
    metrics,
  });

  useOverlayVisibility(pillRef);

  // Auto-scroll the transcript to the latest line as it grows. transcriptRef is
  // a stable ref (intentionally omitted); the effect re-runs on content change.
  // biome-ignore lint/correctness/useExhaustiveDependencies: transcriptRef is a stable ref; finalText/partialText drive the scroll timing
  useEffect(() => {
    const el = transcriptRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [hasHint, state.finalText, state.partialText]);

  return (
    <main className="overlay-main flex h-full w-full items-end justify-center overflow-hidden p-[28px_32px_40px] [font-family:var(--font-family-ui)]">
      <section className="overlay-surface relative flex max-h-full w-full max-w-[680px] items-end justify-center overflow-visible">
        <div
          className="shortcut-row"
          data-visible={state.shortcuts.length > 0}
          aria-hidden={state.shortcuts.length === 0}
        >
          {displayedShortcuts.map((shortcut) => (
            <span
              key={`${shortcut.key}-${shortcut.label}`}
              className="inline-flex items-center gap-1 whitespace-nowrap"
            >
              <kbd>{shortcut.key}</kbd>
              <span>{shortcut.label}</span>
            </span>
          ))}
        </div>
        <div ref={pillRef} className="overlay-group relative max-w-full">
          <div
            data-wrap={wrap ? "multi" : "single"}
            className={`pill relative inline-flex w-full items-center gap-3 overflow-hidden px-4 ${
              wrap || isError ? "rounded-2xl min-h-[40px] py-2" : "h-[40px] rounded-full"
            } ${!hasHint && (success || !hasText) ? "justify-center" : ""} ${centerText ? "status-pill" : ""}`}
          >
            {showIcon &&
              (success ? (
                <Check className="h-4 w-4 shrink-0" aria-label={t("输出成功")} />
              ) : (
                <Info className="h-4 w-4 shrink-0" aria-hidden="true" />
              ))}
            {showWave && <Waveform active={waveActive} metrics={metrics} />}
            {(hasHint || (!success && hasText)) && (
              <div
                className={`min-w-0 flex-1 text-[14px] font-medium leading-[1.3] ${centerText ? "text-center" : ""}`}
              >
                {hasHint ? (
                  <span
                    className={`block ${isError ? "whitespace-pre-wrap break-words" : "truncate"} ${shouldPulse ? "pulse-text" : ""}`}
                  >
                    {visibleHint}
                  </span>
                ) : (
                  <div
                    ref={transcriptRef}
                    className={
                      wrap
                        ? "block overflow-hidden whitespace-normal break-words leading-[1.32] [max-height:calc(1.32em*var(--overlay-max-lines,3)+1px)] [mask-image:linear-gradient(180deg,transparent_0%,#000_30%)]"
                        : "truncate"
                    }
                    style={{ "--overlay-max-lines": metrics.max_text_lines } as CSSProperties}
                  >
                    <span className="text-inherit">{state.finalText}</span>
                    <span className="opacity-55">{state.partialText}</span>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </section>
      <div
        ref={measureRef}
        className="measure-text pointer-events-none fixed left-[-9999px] top-[-9999px] max-w-none whitespace-nowrap font-medium text-[14px] leading-[1.3] [font-family:var(--font-family-ui)] invisible"
      />
    </main>
  );
}
