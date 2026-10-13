import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OverlayEvent } from "@/overlay/bridge";

const { listen } = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@/overlay/bridge", () => ({ onOverlayEvent: listen }));

import { useOverlayState } from "@/overlay/useOverlayState";

let listener: (event: OverlayEvent) => void;
beforeEach(() => {
  vi.useFakeTimers();
  listen.mockImplementation((callback) => {
    listener = callback;
    return vi.fn();
  });
});
afterEach(() => vi.useRealTimers());
function emit(type: string, payload: Record<string, unknown> = {}) {
  act(() => listener({ type, payload }));
}
describe("overlay event lifecycle", () => {
  it("leaves error lifetime to the backend, including repeated failures", () => {
    const { result } = renderHook(() => useOverlayState());
    emit("hint", { level: "error", text: "连接失败" });
    emit("shortcuts", { shortcuts: [{ key: "Fn", label: "重试" }] });
    act(() => vi.advanceTimersByTime(4000));
    emit("hint", { level: "error", text: "认证失败" });
    act(() => vi.advanceTimersByTime(2000));
    expect(result.current.state.hintText).toBe("认证失败");
    expect(result.current.state.shortcuts).toHaveLength(1);
  });
  it("only animates audio during recording and resets on a new session", () => {
    const { result } = renderHook(() => useOverlayState());
    emit("state", { state: "recording" });
    emit("audio:level", { active: true });
    expect(result.current.waveActive).toBe(true);
    emit("reset");
    emit("audio:level", { active: true });
    expect(result.current.waveActive).toBe(false);
  });
  it("immediately accepts error shortcuts and replaces them with success feedback", () => {
    const { result } = renderHook(() => useOverlayState());
    emit("shortcuts", { shortcuts: [{ key: "Enter", label: "输出原文" }] });
    expect(result.current.state.shortcuts).toHaveLength(1);
    emit("shortcuts", { shortcuts: [] });
    emit("hint", { variant: "success", text: "" });
    emit("state", { state: "idle" });
    expect(result.current.state).toMatchObject({
      shortcuts: [],
      hintVariant: "success",
      hintText: "",
    });
  });
});
