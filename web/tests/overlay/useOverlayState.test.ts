import { describe, expect, it } from "vitest";
import { INITIAL_OVERLAY_STATE } from "@/overlay/types";
import { reducer } from "@/overlay/useOverlayState";

describe("overlay reducer", () => {
  it("preserves success feedback when delivery settles to idle", () => {
    const success = reducer(INITIAL_OVERLAY_STATE, {
      type: "hint",
      variant: "success",
      text: "已保存到剪切板",
    });
    const next = reducer(success, { type: "state", state: "idle" });
    expect(next.hintVariant).toBe("success");
    expect(next.hintText).toBe("已保存到剪切板");
  });
  it("preserves an error and its shortcuts when settling to idle", () => {
    const error = reducer(INITIAL_OVERLAY_STATE, {
      type: "hint",
      level: "error",
      text: "连接失败",
    });
    const armed = reducer(error, { type: "shortcuts", shortcuts: [{ key: "Fn", label: "重试" }] });
    expect(reducer(armed, { type: "state", state: "idle" })).toMatchObject({
      hintText: "连接失败",
      shortcuts: armed.shortcuts,
    });
  });
  it("clears error feedback and shortcuts when a retry starts", () => {
    const error = {
      ...INITIAL_OVERLAY_STATE,
      hintLevel: "error" as const,
      hintText: "连接失败",
      shortcuts: [{ key: "Fn", label: "重试" }],
    };
    expect(reducer(error, { type: "state", state: "finishing" })).toMatchObject({
      hintText: "",
      hintLevel: "info",
      shortcuts: [],
    });
  });
  it("resets recording hints while preserving appearance", () => {
    const state = {
      ...INITIAL_OVERLAY_STATE,
      theme: "light" as const,
      shortcuts: [{ key: "Esc", label: "取消" }],
    };
    expect(reducer(state, { type: "reset" })).toMatchObject({ theme: "light", shortcuts: [] });
  });
});
