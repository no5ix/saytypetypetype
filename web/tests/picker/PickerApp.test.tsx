import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PickerOptionView, PickerView } from "@/picker/bridge";

const h = vi.hoisted(() => ({
  listeners: new Set<(view: unknown) => void>(),
  view: null as unknown,
  selectPickerOption: vi.fn(async () => {}),
  cancelPicker: vi.fn(async () => {}),
}));

vi.mock("@/picker/bridge", () => ({
  getPickerState: () => Promise.resolve(h.view),
  onPickerState: (listener: (view: unknown) => void) => {
    h.listeners.add(listener);
    return () => h.listeners.delete(listener);
  },
  selectPickerOption: h.selectPickerOption,
  cancelPicker: h.cancelPicker,
}));

import { PickerApp } from "@/picker/PickerApp";

function option(
  overrides: Partial<PickerOptionView> & Pick<PickerOptionView, "id">,
): PickerOptionView {
  return {
    title: overrides.id,
    hotkey: "",
    kind: "template",
    status: "ready",
    text: `text for ${overrides.id}`,
    error: null,
    ...overrides,
  };
}

function view(optionOverrides: PickerOptionView[], sessionId = 1): PickerView {
  return {
    sessionId,
    options: optionOverrides,
    delivering: false,
    allCompleted: true,
    error: null,
    theme: "dark",
    targetApp: "Safari",
  };
}

const DEFAULT_OPTIONS: PickerOptionView[] = [
  option({ id: "original", title: "原文", hotkey: "R ⌥", kind: "original" }),
  option({ id: "polish", title: "润色", hotkey: "U" }),
  option({ id: "summary", title: "要点", hotkey: "V" }),
];

function scrollRegion(container: HTMLElement): HTMLElement {
  const region = container.querySelector<HTMLElement>("[data-picker-scroll]");
  if (!region) throw new Error("scroll region not found");
  return region;
}

async function renderPicker(nextView: PickerView) {
  h.view = nextView;
  const result = render(<PickerApp />);
  await waitFor(() => expect(result.container.querySelector("section")).not.toBeNull());
  return result;
}

beforeEach(() => {
  h.listeners.clear();
  h.selectPickerOption.mockClear();
  h.cancelPicker.mockClear();
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
  Object.defineProperty(Element.prototype, "scrollTo", {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("PickerApp layout", () => {
  it("keeps the header outside the scrolling card region", async () => {
    const { container } = await renderPicker(view(DEFAULT_OPTIONS));
    const section = container.querySelector("section");
    const header = container.querySelector("header");
    const region = scrollRegion(container);

    expect(section).not.toBeNull();
    expect(header).not.toBeNull();
    // The header owns layout space as a flex item: it is a direct child of the
    // panel and is never nested inside the scroll container.
    expect(header?.parentElement).toBe(section);
    expect(region.parentElement).toBe(section);
    expect(region.contains(header)).toBe(false);
    // It cannot be squeezed away when the window is short, it paints an opaque
    // background over the scroll cut line, and it sits above card hover lifts.
    expect(header?.className).toContain("shrink-0");
    expect(header?.className).toContain("bg-surface-card");
    expect(header?.className).toContain("relative");
    expect(header?.className).toContain("z-10");

    // The card region starts below the header with a small visual gap instead of
    // being flush against it, and it owns the scroll behaviour.
    expect(region.className).toContain("mt-1.5");
    expect(region.className).toContain("overflow-y-auto");
    expect(region.className).toContain("overscroll-contain");
    expect(region.className).not.toContain("absolute");
  });

  it("renders every option inside the scroll region", async () => {
    const { container } = await renderPicker(view(DEFAULT_OPTIONS));
    const region = scrollRegion(container);
    const cards = screen.getAllByRole("button", { name: /^(原文|润色|要点)/ });
    expect(cards).toHaveLength(3);
    for (const card of cards) expect(region.contains(card)).toBe(true);
  });

  it("resets a scroll offset left by the previous session", async () => {
    const { container } = await renderPicker(view(DEFAULT_OPTIONS, 1));
    const region = scrollRegion(container);
    region.scrollTop = 120;
    const scrollTo = vi.mocked(Element.prototype.scrollTo);
    // The initial session legitimately starts at the top; only the session
    // change is under test here.
    scrollTo.mockClear();

    await act(async () => {
      for (const listener of h.listeners) listener(view(DEFAULT_OPTIONS, 2));
    });

    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({ top: 0 }));
  });
});

describe("PickerApp hover and selection", () => {
  it("marks hovering as an accent affordance distinct from the original-tint", async () => {
    const { container } = await renderPicker(view(DEFAULT_OPTIONS));
    const polish = container.querySelector<HTMLElement>("button[aria-label^='润色']");
    const original = container.querySelector<HTMLElement>("button[aria-label^='原文']");

    expect(polish?.className).toContain("hover:border-accent");
    expect(polish?.className).not.toContain("hover:border-border-hover");
    // The original option keeps its permanent accent tint so the selected/
    // special option stays distinguishable while another card is hovered.
    expect(original?.className).toContain("border-accent/45");
    expect(original?.className).toContain("bg-accent-soft/30");
  });

  it("does not select anything on hover, but selects on click", async () => {
    const { container } = await renderPicker(view(DEFAULT_OPTIONS));
    const polish = container.querySelector<HTMLElement>("button[aria-label^='润色']");
    expect(polish).not.toBeNull();

    fireEvent.mouseOver(polish as HTMLElement);
    fireEvent.mouseEnter(polish as HTMLElement);
    fireEvent.pointerOver(polish as HTMLElement);
    expect(h.selectPickerOption).not.toHaveBeenCalled();

    fireEvent.click(polish as HTMLElement);
    await waitFor(() => expect(h.selectPickerOption).toHaveBeenCalledWith(1, "polish"));
  });

  it("ignores clicks on cards that are still loading or failed", async () => {
    const options = [
      option({ id: "original", title: "原文", kind: "original" }),
      option({ id: "polish", title: "润色", status: "loading", text: null }),
      option({ id: "summary", title: "要点", status: "failed", text: null, error: "超时" }),
    ];
    const { container } = await renderPicker(view(options));

    fireEvent.click(
      container.querySelector<HTMLElement>("button[aria-label^='润色']") as HTMLElement,
    );
    fireEvent.click(
      container.querySelector<HTMLElement>("button[aria-label^='要点']") as HTMLElement,
    );
    expect(h.selectPickerOption).not.toHaveBeenCalled();

    fireEvent.click(
      container.querySelector<HTMLElement>("button[aria-label^='原文']") as HTMLElement,
    );
    await waitFor(() => expect(h.selectPickerOption).toHaveBeenCalledWith(1, "original"));
  });
});
