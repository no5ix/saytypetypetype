import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OverlayEvent } from "@/overlay/bridge";

const { onEvent } = vi.hoisted(() => ({
  onEvent: vi.fn((_listener: (event: OverlayEvent) => void) => () => {}),
}));

vi.mock("@/overlay/bridge", () => ({ onOverlayEvent: onEvent }));

import { useOverlayState } from "@/overlay/useOverlayState";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("overlay sound activity", () => {
  it("ignores late audio after finishing and starts the next session at rest", () => {
    const { result } = renderHook(() => useOverlayState());
    const emit = (type: string, payload: Record<string, unknown>) => {
      act(() => onEvent.mock.calls[0][0]({ type, payload }));
    };
    emit("audio:level", { active: true });
    expect(result.current.waveActive).toBe(false);
    emit("state", { state: "recording" });
    emit("audio:level", { active: true });
    expect(result.current.waveActive).toBe(true);
    emit("audio:level", { active: false });
    expect(result.current.waveActive).toBe(false);
    emit("audio:level", { active: true });
    emit("state", { state: "finishing" });
    emit("audio:level", { active: true });
    expect(result.current.waveActive).toBe(false);
    emit("state", { state: "recording" });
    expect(result.current.waveActive).toBe(false);
    emit("audio:level", { active: true });
    emit("reset", {});
    expect(result.current.waveActive).toBe(false);
  });
});
