import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OverlayLayoutMetrics } from "@/overlay/bridge";
import { Waveform } from "@/overlay/Waveform";

const metrics = {
  wave_bar_w: 4,
  wave_bar_gap: 2,
  wave_corner_radius: 2,
  wave_min_h: 4,
  wave_max_h: 22,
  wave_durations: [0.85, 0.7, 1.0, 0.8],
  wave_peaks: [0.65, 1.0, 0.85, 0.55],
  wave_settle_seconds: 0.18,
} as OverlayLayoutMetrics;

beforeEach(() => {
  // jsdom has no animation engine; verify lifecycle calls here, visuals manually.
  Object.defineProperty(Element.prototype, "animate", { value: vi.fn(), configurable: true });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  Reflect.deleteProperty(Element.prototype, "animate");
});

describe("waveform animation lifecycle", () => {
  it("keeps loops running across repeated active events and cancels on unmount", () => {
    const cancel = vi.fn();
    const animate = vi
      .spyOn(Element.prototype, "animate")
      .mockReturnValue({ cancel } as unknown as Animation);
    const { container, rerender, unmount } = render(<Waveform active metrics={metrics} />);
    const waveform = container.firstElementChild as HTMLElement;
    expect(waveform.classList.contains("items-center")).toBe(true);
    expect(waveform.style.height).toBe("22px");
    for (const bar of waveform.children) {
      expect((bar as HTMLElement).style.width).toBe("4px");
      expect((bar as HTMLElement).style.borderRadius).toBe("2px");
    }
    expect(animate).toHaveBeenCalledTimes(4);
    expect(
      animate.mock.calls.map((call) => (call[1] as KeyframeAnimationOptions).duration),
    ).toEqual([850, 700, 1000, 800]);
    rerender(<Waveform active metrics={metrics} />);
    expect(animate).toHaveBeenCalledTimes(4);
    unmount();
    expect(cancel).toHaveBeenCalledTimes(4);
  });

  it("returns from the currently displayed height to resting bars after sound stops", () => {
    const cancel = vi.fn();
    const animate = vi
      .spyOn(Element.prototype, "animate")
      .mockReturnValue({ cancel } as unknown as Animation);
    const { rerender } = render(<Waveform active metrics={metrics} />);
    vi.spyOn(window, "getComputedStyle").mockReturnValue({
      height: "15px",
    } as CSSStyleDeclaration);
    rerender(<Waveform active={false} metrics={metrics} />);
    expect(cancel).toHaveBeenCalledTimes(4);
    expect(animate).toHaveBeenLastCalledWith(
      [{ height: "15px" }, { height: "4px" }],
      expect.objectContaining({ duration: 180, iterations: 1 }),
    );
  });
});
