/// <reference types="node" />
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PickerOptionView, PickerView } from "@/picker/bridge";

import { PickerApp } from "@/picker/PickerApp";

/**
 * The stylesheet the picker window actually loads. Asserted as text so the
 * appearance rules (no outer shadow, theme-aware 1px outline, motion budget) are
 * checked against the source of truth instead of a jsdom approximation, which
 * neither applies external stylesheets nor real layout.
 */
const css = (() => {
  const candidates = ["src/picker/picker.css", "web/src/picker/picker.css"].map((relative) =>
    resolve(process.cwd(), relative),
  );
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) throw new Error(`picker.css not found near ${process.cwd()}`);
  return readFileSync(found, "utf8");
})();

const h = vi.hoisted(() => ({
  stateListeners: new Set<(view: unknown) => void>(),
  closingListeners: new Set<(signal: { closing?: boolean }) => void>(),
  view: null as unknown,
  selectPickerOption: vi.fn(async () => {}),
  cancelPicker: vi.fn(async () => {}),
}));

vi.mock("@/picker/bridge", () => ({
  getPickerState: () => Promise.resolve(h.view),
  onPickerState: (listener: (view: unknown) => void) => {
    h.stateListeners.add(listener);
    return () => h.stateListeners.delete(listener);
  },
  onPickerClosing: (listener: (signal: { closing?: boolean }) => void) => {
    h.closingListeners.add(listener);
    return () => h.closingListeners.delete(listener);
  },
  selectPickerOption: h.selectPickerOption,
  cancelPicker: h.cancelPicker,
}));

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

function view(optionOverrides: PickerOptionView[], sessionId = 1, columns?: number): PickerView {
  return {
    sessionId,
    options: optionOverrides,
    delivering: false,
    allCompleted: true,
    error: null,
    theme: "dark",
    targetApp: "Safari",
    ...(columns ? { columns } : {}),
  };
}

const OPTIONS: PickerOptionView[] = [
  option({ id: "original", title: "原文", hotkey: "R ⌥", kind: "original" }),
  option({ id: "polish", title: "润色", hotkey: "U" }),
  option({ id: "summary", title: "要点", hotkey: "V" }),
];

function panel(container: HTMLElement): HTMLElement {
  const section = container.querySelector<HTMLElement>("section");
  if (!section) throw new Error("picker panel not rendered");
  return section;
}

function scrollRegion(container: HTMLElement): HTMLElement {
  const region = container.querySelector<HTMLElement>("[data-picker-scroll]");
  if (!region) throw new Error("scroll region not rendered");
  return region;
}

async function renderPicker(nextView: PickerView) {
  h.view = nextView;
  const result = render(<PickerApp />);
  await waitFor(() => expect(result.container.querySelector("section")).not.toBeNull());
  return result;
}

function emitState(next: PickerView) {
  act(() => {
    for (const listener of h.stateListeners) listener(next);
  });
}

function emitClosing(closing = true) {
  act(() => {
    for (const listener of h.closingListeners) listener({ closing });
  });
}

beforeEach(() => {
  h.stateListeners.clear();
  h.closingListeners.clear();
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

describe("PickerApp panel outline and shadow", () => {
  it("draws the panel with a theme-aware border and no drop shadow", () => {
    const surface = css.slice(css.indexOf(".picker-surface {"), css.indexOf("/* Pop-in"));
    expect(surface).toContain("border: 1px solid var(--picker-border)");
    expect(surface).not.toContain("box-shadow");
    // No theme variant adds a shadow back either.
    expect(css).not.toMatch(/\.picker-surface[^{]*\{[^}]*box-shadow/);
  });

  it("outlines dark themes with near-white and light themes with near-black", () => {
    expect(css).toMatch(/--picker-border:\s*rgba\(255,\s*255,\s*255,\s*0\.\d+\)/);
    expect(css).toMatch(/\[data-theme="light"\][^{]*\{[^}]*--picker-border:\s*rgba\(0,\s*0,\s*0,/s);
    // The border reuses the app's existing theme switch, not a new theme system.
    expect(css).toContain('[data-theme="light"] .picker-root');
  });

  it("keeps the border on the panel element and off the placeholder pill", async () => {
    h.view = null;
    const placeholder = render(<PickerApp />);
    await waitFor(() => expect(placeholder.container.firstChild).not.toBeNull());
    const pill = placeholder.container.querySelector<HTMLElement>(".picker-surface");
    expect(pill, "the waiting pill uses the same outlined surface").not.toBeNull();
    expect(pill?.className).not.toContain("shadow");

    const { container } = await renderPicker(view(OPTIONS));
    expect(panel(container).className).toContain("picker-surface");
    expect(panel(container).className).not.toContain("shadow");
  });
});

describe("PickerApp pop-in and pop-out", () => {
  it("animates within the 120-180ms budget and drops the scale for reduced motion", () => {
    const durations = [...css.matchAll(/--picker-pop-(in|out):\s*(\d+)ms/g)].map((m) =>
      Number(m[2]),
    );
    expect(durations.length).toBeGreaterThanOrEqual(2);
    for (const duration of durations) {
      expect(duration).toBeGreaterThanOrEqual(120);
      expect(duration).toBeLessThanOrEqual(180);
    }
    expect(css).toContain("animation: picker-in");
    expect(css).toContain("@keyframes picker-in");
    expect(css).toMatch(/scale\(var\(--picker-pop-scale\)\)/);

    const reduced = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
    expect(reduced).toContain("--picker-pop-scale: 1");
  });

  it("marks the panel per session so the entrance animation replays", async () => {
    const { container } = await renderPicker(view(OPTIONS, 7));
    const first = panel(container);
    expect(first.getAttribute("data-picker-state")).toBe("open");

    emitState(view(OPTIONS, 8));
    const second = panel(container);
    expect(second.getAttribute("data-picker-state")).toBe("open");
    // A new DOM node for a new session restarts the CSS keyframes.
    expect(second.isSameNode(first)).toBe(false);
  });

  it("plays the exit when the backend announces the close and resets for the next session", async () => {
    const { container } = await renderPicker(view(OPTIONS, 1));
    expect(panel(container).getAttribute("data-picker-state")).toBe("open");

    emitClosing();
    expect(panel(container).getAttribute("data-picker-state")).toBe("closing");

    // Re-opening while an exit is in flight must return to the open state, so the
    // window can never stay invisible-but-shown.
    emitState(view(OPTIONS, 2));
    expect(panel(container).getAttribute("data-picker-state")).toBe("open");
  });
});

describe("PickerApp layout", () => {
  it("keeps header, cards and footer as separate rows of the panel", async () => {
    const { container } = await renderPicker(view(OPTIONS));
    const section = panel(container);
    const header = container.querySelector("header");
    const footer = container.querySelector("footer");
    const region = scrollRegion(container);

    expect(header?.parentElement).toBe(section);
    expect(footer?.parentElement).toBe(section);
    expect(region.parentElement).toBe(section);
    expect(region.contains(header)).toBe(false);
    expect(region.contains(footer)).toBe(false);

    // Nothing may collapse when the window is short.
    expect(header?.className).toContain("shrink-0");
    expect(footer?.className).toContain("shrink-0");

    // The header owns its own space and paints over the scroll cut line: opaque
    // background, positioned above the cards, with its own separating border.
    expect(header?.className).toContain("relative");
    expect(header?.className).toContain("z-10");
    expect(header?.className).toContain("bg-surface-card");
    expect(header?.className).toContain("border-b");

    // The card region owns the remaining space, starts below the header with a
    // small gap, and owns the scroll behaviour (it is not absolutely positioned).
    expect(region.className).toContain("min-h-0");
    expect(region.className).toContain("flex-1");
    expect(region.className).toContain("mt-1.5");
    expect(region.className).toContain("overflow-y-auto");
    expect(region.className).toContain("overscroll-contain");
    expect(region.className).not.toContain("absolute");
  });

  it("renders every option inside the scroll region and resets it per session", async () => {
    const { container } = await renderPicker(view(OPTIONS, 1));
    const region = scrollRegion(container);
    const cards = screen.getAllByRole("button", { name: /^(原文|润色|要点)/ });
    expect(cards).toHaveLength(3);
    for (const card of cards) expect(region.contains(card)).toBe(true);

    region.scrollTop = 120;
    const scrollTo = vi.mocked(Element.prototype.scrollTo);
    // The initial session legitimately starts at the top; only the session change
    // is under test here.
    scrollTo.mockClear();

    emitState(view(OPTIONS, 2));
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({ top: 0 }));
  });
});

describe("PickerApp hover and selection", () => {
  it("marks hovering as an accent affordance distinct from the original-tint", async () => {
    const { container } = await renderPicker(view(OPTIONS));
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
    const { container } = await renderPicker(view(OPTIONS));
    const polish = container.querySelector<HTMLElement>("button[aria-label^='润色']");
    expect(polish).not.toBeNull();

    fireEvent.mouseOver(polish as HTMLElement);
    fireEvent.mouseEnter(polish as HTMLElement);
    fireEvent.pointerOver(polish as HTMLElement);
    expect(h.selectPickerOption).not.toHaveBeenCalled();

    fireEvent.click(polish as HTMLElement);
    await waitFor(() => expect(h.selectPickerOption).toHaveBeenCalledWith(1, "polish"));
  });

  it("still pastes the clicked version", async () => {
    const { container } = await renderPicker(view(OPTIONS, 4));
    fireEvent.click(
      container.querySelector<HTMLElement>("button[aria-label^='润色']") as HTMLElement,
    );
    await waitFor(() => expect(h.selectPickerOption).toHaveBeenCalledWith(4, "polish"));
  });

  it("still cancels through the Esc control", async () => {
    await renderPicker(view(OPTIONS, 5));
    fireEvent.click(screen.getByRole("button", { name: /Esc 取消/ }));
    await waitFor(() => expect(h.cancelPicker).toHaveBeenCalledWith(5));
  });

  it("ignores clicks while a delivery is in flight or a card is not ready", async () => {
    const options = [
      option({ id: "original", title: "原文", kind: "original" }),
      option({ id: "polish", title: "润色", status: "loading", text: null }),
      option({ id: "summary", title: "要点", status: "failed", text: null, error: "超时" }),
    ];
    const { container } = await renderPicker(view(options, 6));

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
    await waitFor(() => expect(h.selectPickerOption).toHaveBeenCalledWith(6, "original"));
  });
});

describe("PickerApp immediate dismissal", () => {
  it("begins the pop-out in the same frame a card is clicked", async () => {
    // Keep the selection IPC open: the dismissal must not wait for it.
    let resolveSelect: () => void = () => {};
    h.selectPickerOption.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          resolveSelect = resolve;
        }),
    );
    const { container } = await renderPicker(view(OPTIONS, 9));

    fireEvent.click(
      container.querySelector<HTMLElement>("button[aria-label^='润色']") as HTMLElement,
    );

    // Asserted synchronously: the panel is already exiting while the paste
    // work is still in flight (previously the exit only started after it).
    expect(panel(container).getAttribute("data-picker-state")).toBe("closing");
    expect(h.selectPickerOption).toHaveBeenCalledWith(9, "polish");
    resolveSelect();
  });

  it("begins the pop-out on the Esc control without pasting anything", async () => {
    const { container } = await renderPicker(view(OPTIONS, 10));

    fireEvent.click(screen.getByRole("button", { name: /Esc 取消/ }));

    expect(panel(container).getAttribute("data-picker-state")).toBe("closing");
    expect(h.cancelPicker).toHaveBeenCalledWith(10);
    expect(h.selectPickerOption).not.toHaveBeenCalled();
  });

  it("pastes only once when selection events repeat while closing", async () => {
    const { container } = await renderPicker(view(OPTIONS, 11));
    const polish = container.querySelector<HTMLElement>(
      "button[aria-label^='润色']",
    ) as HTMLElement;

    fireEvent.click(polish);
    fireEvent.click(polish);
    fireEvent.click(polish);

    expect(h.selectPickerOption).toHaveBeenCalledTimes(1);
    expect(panel(container).getAttribute("data-picker-state")).toBe("closing");
  });

  it("returns to open when a failed delivery re-arms the same session", async () => {
    const { container } = await renderPicker(view(OPTIONS, 12));

    emitClosing(true);
    expect(panel(container).getAttribute("data-picker-state")).toBe("closing");

    // The backend re-showed the window after a failed paste: same session id,
    // so only an explicit reopen signal can stop the exit animation.
    emitClosing(false);
    expect(panel(container).getAttribute("data-picker-state")).toBe("open");
  });

  it("returns the panel to open when the selection IPC itself fails", async () => {
    h.selectPickerOption.mockRejectedValueOnce(new Error("ipc down"));
    const { container } = await renderPicker(view(OPTIONS, 13));

    fireEvent.click(
      container.querySelector<HTMLElement>("button[aria-label^='润色']") as HTMLElement,
    );
    expect(panel(container).getAttribute("data-picker-state")).toBe("closing");

    await waitFor(() => expect(panel(container).getAttribute("data-picker-state")).toBe("open"));
    expect(h.cancelPicker).not.toHaveBeenCalled();
  });
});

describe("PickerApp native column policy", () => {
  it("uses the payload column count over a stale window width", async () => {
    // 652px would fall into the legacy two-column tier; the native layout for
    // this session says three, and the native number wins because it is the
    // number the window was actually sized for.
    vi.stubGlobal("innerWidth", 652);
    const { container } = await renderPicker(view(OPTIONS, 14, 3));
    const grid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    expect(grid?.style.gridTemplateColumns).toContain("repeat(3");

    // Payloads without the field still fall back to the width heuristic.
    emitState(view(OPTIONS, 15));
    const nextGrid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    expect(nextGrid?.style.gridTemplateColumns).toContain("repeat(2");
  });

  it("keeps every card on one row when the payload says one row", async () => {
    const { container } = await renderPicker(view(OPTIONS, 16, 3));
    const cards = screen.getAllByRole("button", { name: /^(原文|润色|要点)/ });
    expect(cards).toHaveLength(3);
    const grid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    // Three columns × three cards = a single horizontal row.
    expect(grid?.style.gridTemplateColumns).toBe("repeat(3, minmax(0, 1fr))");
  });
});
