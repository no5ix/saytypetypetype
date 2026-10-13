import { useEffect, useRef } from "react";
import type { OverlayLayoutMetrics } from "./bridge";

/** WebView animations run independently of the ~10 Hz audio event cadence. */
export function Waveform({ active, metrics }: { active: boolean; metrics: OverlayLayoutMetrics }) {
  const bars = useRef<(HTMLSpanElement | null)[]>([]);
  const animations = useRef<Animation[]>([]);
  const rest = metrics.wave_min_h;

  useEffect(() => {
    bars.current.forEach((bar, index) => {
      if (!bar) return;
      const current = getComputedStyle(bar).height;
      animations.current[index]?.cancel();
      const peak = metrics.wave_max_h * metrics.wave_peaks[index];
      animations.current[index] = bar.animate(
        active
          ? [rest, peak, peak * 0.45, rest].map((height) => ({
              height: `${height}px`,
              easing: "ease-in-out",
            }))
          : [{ height: current }, { height: `${rest}px` }],
        {
          duration: (active ? metrics.wave_durations[index] : metrics.wave_settle_seconds) * 1000,
          iterations: active ? Number.POSITIVE_INFINITY : 1,
          easing: active ? "linear" : "ease-out",
          fill: "forwards",
        },
      );
    });
  }, [active, metrics, rest]);

  useEffect(
    () => () => {
      animations.current.forEach((animation) => {
        animation.cancel();
      });
    },
    [],
  );

  return (
    <span
      className="flex shrink-0 items-center"
      style={{ height: metrics.wave_max_h, gap: metrics.wave_bar_gap }}
      aria-hidden="true"
    >
      {metrics.wave_peaks.map((peak, index) => (
        <span
          key={peak}
          ref={(bar) => {
            bars.current[index] = bar;
          }}
          className="bg-overlay-accent"
          style={{
            width: metrics.wave_bar_w,
            height: rest,
            borderRadius: metrics.wave_corner_radius,
          }}
        />
      ))}
    </span>
  );
}
