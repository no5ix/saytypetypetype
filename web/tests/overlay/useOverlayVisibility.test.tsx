import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OverlayEvent } from "@/overlay/bridge";
import { useOverlayVisibility } from "@/overlay/useOverlayVisibility";

const { completeHide, listen } = vi.hoisted(() => ({
  completeHide: vi.fn(() => Promise.resolve()),
  listen: vi.fn(),
}));
vi.mock("@/overlay/bridge", () => ({
  completeOverlayHide: completeHide,
  onOverlayEvent: listen,
}));

let listener: (event: OverlayEvent) => void;
let finishes: (() => void)[];
let animate: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  finishes = [];
  listen.mockImplementation((callback) => {
    listener = callback;
    return vi.fn();
  });
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  // Completion is deliberately controllable, including completion after cancellation.
  animate = vi.fn(() => ({
    cancel: vi.fn(),
    finished: new Promise<void>((resolve) => finishes.push(resolve)),
  }));
});
afterEach(() => vi.unstubAllGlobals());

function mount() {
  const pill = document.createElement("div");
  Object.defineProperty(pill, "animate", { value: animate });
  return renderHook(() => useOverlayVisibility({ current: pill }));
}
function visibility(visible: boolean, generation: number) {
  act(() => listener({ type: "visibility", payload: { visible, generation } }));
}

describe("overlay visibility animation", () => {
  it("waits for the exit animation before hiding the native window", async () => {
    mount();
    visibility(true, 1);
    visibility(false, 2);
    expect(completeHide).not.toHaveBeenCalled();
    await act(async () => finishes[1]());
    expect(completeHide).toHaveBeenCalledWith(2);
    expect(animate.mock.calls[1][1]).toMatchObject({ duration: 120 });
  });

  it("ignores an old exit completion when the overlay has reopened", async () => {
    mount();
    visibility(true, 1);
    visibility(false, 2);
    visibility(true, 3);
    await act(async () => finishes[1]());
    expect(completeHide).not.toHaveBeenCalled();
    expect(animate.mock.results[1].value.cancel).toHaveBeenCalledOnce();
    expect(animate.mock.calls[2][1]).toMatchObject({ duration: 160 });
  });

  it("honors reduced motion and still completes the hide", async () => {
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    mount();
    visibility(false, 1);
    expect(animate.mock.calls[0][1]).toMatchObject({ duration: 0 });
    await act(async () => finishes[0]());
    expect(completeHide).toHaveBeenCalledWith(1);
  });
});
