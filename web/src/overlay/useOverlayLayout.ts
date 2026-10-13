import { useEffect, useRef, useState } from "react";
import type { OverlayLayoutMetrics } from "@/overlay/bridge";
import type { AppState } from "./types";

export interface OverlayLayoutInput {
  finalText: string;
  partialText: string;
  visibleHintText: string;
  /** Reserve a left slot only for error and success icons. */
  showIcon?: boolean;
  appState: AppState;
  /** Reconnection warnings hide the recording waveform. */
  hintLevel?: string;
  /** Layout constants fetched from the backend (overlay::shared) so the Windows
   * pill sizes identically to the macOS native pill. */
  metrics: OverlayLayoutMetrics;
}

export interface UseOverlayLayout {
  measureRef: React.RefObject<HTMLDivElement | null>;
  pillRef: React.RefObject<HTMLDivElement | null>;
  transcriptRef: React.RefObject<HTMLDivElement | null>;
  wrap: boolean;
}

/**
 * Measures the transcript/hint text with a hidden node and sizes the pill to
 * fit (single-line ellipsis vs 3-line wrap), with a sticky "locked" width while
 * recording/finishing to prevent jitter. Chrome constants come from the backend
 * (shared::LayoutMetrics) — only text width is measured here (DOM font metrics
 * can't move off the frontend). Widths are applied directly to the pill element
 * (no re-render per frame); only the wrap toggle goes through React state.
 */
export function useOverlayLayout(input: OverlayLayoutInput): UseOverlayLayout {
  const measureRef = useRef<HTMLDivElement | null>(null);
  const pillRef = useRef<HTMLDivElement | null>(null);
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const layoutWidthRef = useRef(0);
  const layoutWrapRef = useRef(false);
  const renderedWidthRef = useRef(0);
  const rafRef = useRef(0);
  const [wrap, setWrap] = useState(false);

  useEffect(() => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      const measure = measureRef.current;
      const pill = pillRef.current;
      if (!measure || !pill) return;

      const m = input.metrics;
      const hasText = !input.showIcon && Boolean(input.finalText || input.partialText);
      const hintText = input.visibleHintText;
      const hasHint = Boolean(hintText);
      const shouldMeasureHintOnly = hasHint;

      if (!hasText && !hasHint) {
        const indicator = input.showIcon
          ? m.indicator_w
          : input.appState === "recording" &&
              input.hintLevel !== "warn" &&
              input.hintLevel !== "error"
            ? m.wave_area_w
            : 0;
        pill.style.width = `${m.pad_left + indicator + m.pad_right}px`;
        renderedWidthRef.current = 0;
        layoutWidthRef.current = 0;
        layoutWrapRef.current = false;
        setWrap(false);
        return;
      }

      let measuredWidth = 0;
      if (hasText && !shouldMeasureHintOnly) {
        measure.textContent = `${input.finalText}${input.partialText}`.trim();
        measuredWidth = Math.ceil(measure.getBoundingClientRect().width);
      }
      let hintWidth = 0;
      if (hasHint) {
        measure.textContent = hintText;
        hintWidth = Math.ceil(measure.getBoundingClientRect().width);
      }

      // Only errors, success, and the recording waveform reserve an icon slot.
      const reconnecting = input.appState === "recording" && input.hintLevel === "warn";
      const leftSlotW = input.showIcon
        ? m.indicator_w
        : input.appState === "recording" && !reconnecting
          ? m.wave_area_w
          : 0;
      // Chrome mirrors src-tauri/src/overlay/shared.rs (single source of truth).
      const chrome = m.pad_left + m.pad_right + leftSlotW + (leftSlotW ? m.gap : 0);
      const lockLayout =
        !input.showIcon &&
        hasText &&
        !shouldMeasureHintOnly &&
        (input.appState === "recording" || input.appState === "finishing");
      const shouldWrap =
        !input.showIcon &&
        hasText &&
        !shouldMeasureHintOnly &&
        (layoutWrapRef.current || measuredWidth > m.single_line_limit);
      const textWidth = Math.max(measuredWidth, hintWidth) + (hasHint ? 2 : m.text_slack);
      const nextWidth = shouldWrap
        ? m.multi_line_width + chrome
        : Math.min(m.single_line_limit + chrome, Math.max(m.min_pill_w, textWidth + chrome));

      if (!lockLayout) {
        layoutWidthRef.current = nextWidth;
        layoutWrapRef.current = shouldWrap;
      } else {
        layoutWidthRef.current = Math.max(layoutWidthRef.current || 0, nextWidth);
        layoutWrapRef.current = layoutWrapRef.current || shouldWrap;
      }
      setWrap(layoutWrapRef.current);

      let width = layoutWidthRef.current || nextWidth;
      if (width !== renderedWidthRef.current) {
        renderedWidthRef.current = width;
        pill.style.width = `${width}px`;
      }

      // Recover any sub-pixel overflow the estimate missed (single-line only).
      if (!layoutWrapRef.current && !shouldMeasureHintOnly && transcriptRef.current) {
        const overflow = transcriptRef.current.scrollWidth - transcriptRef.current.clientWidth;
        if (overflow > 0) {
          width += overflow + 6;
          layoutWidthRef.current = Math.max(layoutWidthRef.current || 0, width);
          renderedWidthRef.current = width;
          pill.style.width = `${width}px`;
        }
      }
    });
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [
    input.finalText,
    input.partialText,
    input.visibleHintText,
    input.showIcon,
    input.appState,
    input.hintLevel,
    input.metrics,
  ]);

  return {
    measureRef,
    pillRef,
    transcriptRef,
    wrap:
      wrap &&
      !input.showIcon &&
      !input.visibleHintText &&
      Boolean(input.finalText || input.partialText),
  };
}
