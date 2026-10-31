import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OverlayEvent } from "@/overlay/bridge";

const { listeners } = vi.hoisted(() => ({ listeners: new Set<(event: OverlayEvent) => void>() }));
vi.mock("@/overlay/bridge", () => ({
  onOverlayEvent: (listener: (event: OverlayEvent) => void) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  getOverlayLayoutMetrics: () => Promise.reject(new Error("fallback metrics")),
  completeOverlayHide: vi.fn(),
  getUiLanguage: () => Promise.resolve("zh-CN"),
  onLanguageChanged: () => () => {},
}));

import { OverlayApp } from "@/overlay/OverlayApp";

const frames = new Map<number, FrameRequestCallback>();
let frameId = 0;
function flushLayout() {
  act(() => {
    const callbacks = [...frames.values()];
    frames.clear();
    for (const callback of callbacks) callback(0);
  });
}
beforeEach(() => {
  frames.clear();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++frameId, callback);
    return frameId;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  Object.defineProperty(HTMLElement.prototype, "animate", {
    configurable: true,
    value: vi.fn(() => ({ cancel: vi.fn() })),
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    return { width: (this.textContent?.length || 0) * 14 } as DOMRect;
  });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  Reflect.deleteProperty(HTMLElement.prototype, "animate");
});
function emit(type: string, payload: Record<string, unknown> = {}) {
  act(() => {
    for (const listener of listeners) listener({ type, payload });
  });
}

describe("overlay presentation", () => {
  it("shows provider warnings and preserves a final-confirmation error after returning idle", () => {
    render(<OverlayApp />);
    emit("state", { state: "recording" });
    emit("transcript", { finalText: "已经识别的文字" });
    const warning = "ElevenLabs：服务端警告，当前会话仍在记录";
    emit("hint", { level: "warn", text: warning, variant: "text" });
    expect(screen.getByText(warning)).toBeTruthy();
    emit("state", { state: "finishing" });
    const failure = "ElevenLabs 最终确认失败：额度不足，请重试";
    emit("hint", { level: "error", text: failure, variant: "text" });
    emit("state", { state: "idle" });
    expect(screen.getByText(failure)).toBeTruthy();
    expect(screen.queryByText("识别中")).toBeNull();
  });
  it("renders preparation and polishing as text without indicator slots", () => {
    const { container } = render(<OverlayApp />);
    emit("state", { state: "connecting" });
    expect(screen.getByText("准备中")).toBeTruthy();
    expect(container.querySelector("svg")).toBeNull();
    expect(container.querySelector(".pill")?.classList.contains("status-pill")).toBe(true);
    emit("state", { state: "finishing" });
    emit("hint", { variant: "progress", text: "润色中" });
    expect(screen.getByText("润色中")).toBeTruthy();
    expect(container.querySelector("svg")).toBeNull();
  });
  it("renders only the centered waveform before a transcript arrives", () => {
    const { container } = render(<OverlayApp />);
    emit("state", { state: "recording" });
    expect(container.querySelector(".pill")?.classList.contains("justify-center")).toBe(true);
    expect(container.querySelector(".pill .flex-1")).toBeNull();
    emit("transcript", { partialText: "测试" });
    expect(screen.getByText("测试")).toBeTruthy();
    expect(container.querySelector(".pill")?.classList.contains("justify-center")).toBe(false);
  });
  it("sizes text and waveform to content and collapses multiline output to a single Check", () => {
    const { container } = render(<OverlayApp />);
    const group = container.querySelector<HTMLElement>(".overlay-group");
    if (!group) throw new Error("Overlay group missing");
    emit("state", { state: "connecting" });
    flushLayout();
    expect(group.style.width).toBe("76px"); // 42 px text + padding and rounding allowance.
    emit("state", { state: "recording" });
    flushLayout();
    expect(group.style.width).toBe("54px"); // 22 px waveform + symmetric padding.
    emit("transcript", { finalText: "长".repeat(50) });
    flushLayout();
    expect(container.querySelector(".pill")?.getAttribute("data-wrap")).toBe("multi");
    emit("state", { state: "finishing" });
    expect(container.querySelector(".pill")?.getAttribute("data-wrap")).toBe("single");
    emit("hint", { variant: "success", text: "" });
    flushLayout();
    expect(group.style.width).toBe("48px");
    expect(container.querySelector(".pill")?.getAttribute("data-wrap")).toBe("single");
    expect(container.querySelector(".pill")?.textContent).toBe("");
  });
  it("measures multiline height at the target width and ignores animated overflow", () => {
    let measuredHeight = 40;
    const widths: number[] = [];
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.classList.contains("pill")) {
        widths.push(Number.parseFloat(this.style.width));
        return { width: Number.parseFloat(this.style.width), height: measuredHeight } as DOMRect;
      }
      return { width: (this.textContent?.length || 0) * 14 } as DOMRect;
    });
    // A transitioning transcript can report large overflow at its current width.
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(5000);
    const { container } = render(<OverlayApp />);
    const group = container.querySelector<HTMLElement>(".overlay-group");
    const surface = container.querySelector<HTMLElement>(".pill");
    if (!group || !surface) throw new Error("Overlay surface missing");
    emit("state", { state: "recording" });
    emit("transcript", { finalText: "长".repeat(50) });
    measuredHeight = 56;
    flushLayout();
    expect(group.style.width).toBe("586px");
    expect(surface.style.height).toBe("56px");
    // Text can add another line without changing the locked width.
    measuredHeight = 74;
    emit("transcript", { finalText: "长".repeat(100) });
    flushLayout();
    expect(group.style.width).toBe("586px");
    expect(surface.style.height).toBe("74px");
    measuredHeight = 40;
    emit("hint", { variant: "success", text: "" });
    flushLayout();
    expect(container.querySelector(".overlay-group")).toBe(group);
    expect(container.querySelector(".pill")).toBe(surface);
    expect(group.style.width).toBe("48px");
    expect(surface.style.height).toBe("40px");
    expect(widths.at(-1)).toBe(48);
    expect(container.querySelector('[aria-hidden="true"].pill')).toBeNull();
  });

  it("renders error shortcuts above the pill without retry buttons or countdowns", () => {
    const { container } = render(<OverlayApp />);
    emit("hint", { level: "error", text: "API Key 无效\n5 秒后输出原文" });
    emit("shortcuts", {
      shortcuts: [
        { key: "Fn", label: "重试" },
        { key: "Esc", label: "取消" },
        { key: "Enter", label: "输出原文" },
      ],
    });
    expect(container.querySelector(".lucide-info")).toBeTruthy();
    expect(container.querySelector(".pill button, .pill-countdown")).toBeNull();
    expect(container.querySelector(".pill kbd")).toBeNull();
    expect(container.querySelectorAll(".shortcut-row kbd")).toHaveLength(3);
    expect(container.querySelector(".overlay-group .shortcut-row")).toBeNull();
    expect(container.querySelector(".lucide-info")?.classList.contains("h-4")).toBe(true);
    expect(screen.getByText("输出原文")).toBeTruthy();
    const row = container.querySelector(".shortcut-row");
    expect(row?.getAttribute("data-visible")).toBe("true");
    emit("shortcuts", { shortcuts: [] });
    expect(row?.getAttribute("data-visible")).toBe("false");
    expect(row?.getAttribute("aria-hidden")).toBe("true");
    expect(row?.querySelectorAll("kbd")).toHaveLength(3); // Retain items for the exit fade.
    emit("shortcuts", { shortcuts: [{ key: "Fn", label: "结束" }] });
    expect(row?.getAttribute("data-visible")).toBe("true");
    expect(row?.querySelectorAll("kbd")).toHaveLength(1);
  });
  it("renders only Check for direct output and adds text for retained clipboard", () => {
    const { container } = render(<OverlayApp />);
    emit("transcript", { finalText: "旧识别文本" });
    emit("state", { state: "finishing" });
    emit("hint", { variant: "success", text: "" });
    expect(container.querySelector(".lucide-check")).toBeTruthy();
    expect(screen.queryByText("识别中")).toBeNull();
    expect(container.querySelector(".pill")?.getAttribute("data-wrap")).toBe("single");
    expect(screen.queryByText("旧识别文本")).toBeNull();
    expect(container.querySelector(".pill")?.textContent).toBe("");
    emit("hint", { variant: "success", text: "已保存到剪切板" });
    expect(screen.getByText("已保存到剪切板")).toBeTruthy();
    emit("state", { state: "idle" });
    expect(screen.getByText("已保存到剪切板")).toBeTruthy();
  });
});
