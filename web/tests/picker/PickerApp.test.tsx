/// <reference types="node" />
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PickerOptionView, PickerView } from "@/picker/bridge";

import { PickerApp } from "@/picker/PickerApp";

/**
 * The stylesheet the picker window actually loads. Asserted as text so the
 * appearance rules (borderless surfaces, theme-specific card colors, motion budget) are
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
  reportPickerLayout: vi.fn(),
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
  reportPickerLayout: h.reportPickerLayout,
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

describe("PickerApp system theme preference", () => {
  it("updates the picker when macOS appearance changes while following the system", async () => {
    const listeners = new Set<() => void>();
    const media = {
      matches,
      media: "(prefers-color-scheme: light)",
      onchange: null,
      addEventListener: vi.fn((_event: string, listener: () => void) => listeners.add(listener)),
      removeEventListener: vi.fn((_event: string, listener: () => void) => listeners.delete(listener)),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    };
    vi.stubGlobal("matchMedia", vi.fn(() => media));

    const systemView = { ...view(OPTIONS), theme: "system" };
    const { unmount } = await renderPicker(systemView);

    // Initial dark system appearance.
    expect(document.documentElement.dataset.theme).toBe("dark");

    // macOS switches to Light while the picker WebView remains mounted.
    media.matches = true;
    act(() => {
      for (const listener of listeners) listener();
    });
    expect(document.documentElement.dataset.theme).toBe("light");

    // And it must follow the reverse transition as well.
    media.matches = false;
    act(() => {
      for (const listener of listeners) listener();
    });
    expect(document.documentElement.dataset.theme).toBe("dark");

    unmount();
    expect(media.removeEventListener).toHaveBeenCalledWith("change", expect.any(Function));
    expect(listeners.size).toBe(0);
  });

  it("does not subscribe to system changes for an explicitly selected theme", async () => {
    const media = {
      matches: false,
      media: "(prefers-color-scheme: light)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    };
    vi.stubGlobal("matchMedia", vi.fn(() => media));

    await renderPicker({ ...view(OPTIONS), theme: "light" });
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(media.addEventListener).not.toHaveBeenCalled();
  });
});

describe("PickerApp borderless color surfaces", () => {
  it("keeps the panel borderless and uses a neutral theme-aware canvas", () => {
    const surface = css.slice(css.indexOf(".picker-surface {"), css.indexOf("/* Pop-in"));
    expect(surface).toContain("border: 0");
    expect(surface).toContain("background: var(--picker-panel-bg)");
    expect(surface).not.toContain("box-shadow");
    const pickerSource = readFileSync(
      resolve(process.cwd(), "src-tauri/src/picker/mod.rs"),
      "utf8",
    );
    expect(pickerSource).toContain(".shadow(true)");
    expect(pickerSource).not.toContain(".shadow(false)");
    expect(css).toContain("--picker-panel-bg: rgba(24, 24, 23, 0.97)");
    expect(css).toContain("--picker-panel-bg: rgba(225, 223, 217, 0.99)");
  });

  it("defines distinct dark and light card colors for all four card types", () => {
    for (const kind of ["original", "polish", "native", "translate"]) {
      expect(css).toContain(`.picker-card-${kind} { background: var(--picker-card-${kind}); }`);
    }
    expect(css).toContain("--picker-card-original: rgba(232, 210, 194, 0.99)");
    expect(css).toContain("--picker-card-polish: rgba(218, 233, 246, 0.99)");
    expect(css).toContain("--picker-card-native: rgba(231, 221, 244, 0.99)");
    expect(css).toContain("--picker-card-translate: rgba(216, 236, 224, 0.99)");
    expect(css).toContain("--picker-card-polish: rgba(35, 57, 76, 0.99)");
    expect(css).toContain("--picker-card-native: rgba(54, 43, 76, 0.99)");
    expect(css).toContain("--picker-card-translate: rgba(35, 66, 53, 0.99)");
    expect(css).toContain(".picker-card {\n  border: 0;");
  });

  it("uses template titles when configured option IDs are opaque", async () => {
    const { container } = await renderPicker(view([
      option({ id: "prompt:opaque-a", title: "General Polishing Dev" }),
      option({ id: "prompt:opaque-b", title: "Native English Dev" }),
      option({ id: "prompt:opaque-c", title: "Translate Dev" }),
    ]));
    const buttonFor = (title: string) =>
      Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
        (button) => button.getAttribute("aria-label") === title,
      );
    expect(buttonFor("General Polishing Dev")?.className).toContain("picker-card-polish");
    expect(buttonFor("Native English Dev")?.className).toContain("picker-card-native");
    expect(buttonFor("Translate Dev")?.className).toContain("picker-card-translate");
  });

  it("gives the Esc cancel control a persistent subtle background", async () => {
    const { container } = await renderPicker(view(OPTIONS));
    const cancel = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("Esc"),
    );
    expect(cancel?.className).toContain("bg-fill-subtle");
    expect(cancel?.className).toContain("cursor-pointer");
    expect(cancel?.getAttribute("data-picker-cancel")).toBe("");
    expect(css).toContain('.picker-cancel[data-native-hovered="true"]');
  });

  it("uses steady resting and hover colors for the light polish and translate cards", () => {
    expect(css).toContain("--picker-card-polish: #bfd6ea");
    expect(css).toContain("--picker-card-polish-hover: #a9c9e3");
    expect(css).toContain("--picker-card-translate: #d1e8d9");
    expect(css).toContain("--picker-card-translate-hover: #c1decf");
    expect(css).toContain("--picker-card-native: #e0d4ef");
    expect(css).toContain("--picker-card-native-hover: #d5c7e8");
    expect(css).toContain('[data-theme="light"] .picker-card-polish[data-native-hovered="true"]');
    expect(css).toContain('[data-theme="light"] .picker-card-native[data-native-hovered="true"]');
    expect(css).toContain('[data-theme="light"] .picker-card-translate[data-native-hovered="true"]');
    expect(css).not.toMatch(/\.picker-card\[data-native-hovered="true"\]\s*\{[^}]*filter:/s);
  });

  it("keeps the waiting surface borderless too", async () => {
    h.view = null;
    const placeholder = render(<PickerApp />);
    await waitFor(() => expect(placeholder.container.firstChild).not.toBeNull());
    const pill = placeholder.container.querySelector<HTMLElement>(".picker-surface");
    expect(pill).not.toBeNull();
    expect(pill?.className).not.toContain("border");
  });
});

describe("PickerApp system theme synchronization", () => {
  it("subscribes to system appearance changes only while the preference is system", async () => {
    const listeners = new Set<(event: MediaQueryListEvent) => void>();
    const media = {
      matches: false,
      addEventListener: vi.fn((name: string, listener: (event: MediaQueryListEvent) => void) => {
        if (name === "change") listeners.add(listener);
      }),
      removeEventListener: vi.fn((name: string, listener: (event: MediaQueryListEvent) => void) => {
        if (name === "change") listeners.delete(listener);
      }),
    };
    vi.stubGlobal("matchMedia", vi.fn(() => media));
    const { unmount } = await renderPicker(view(OPTIONS));
    await waitFor(() => expect(media.addEventListener).toHaveBeenCalledWith("change", expect.any(Function)));
    expect(document.documentElement.dataset.theme).toBe("light");
    media.matches = true;
    act(() => {
      for (const listener of listeners) listener({ matches: true } as MediaQueryListEvent);
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
    unmount();
    expect(media.removeEventListener).toHaveBeenCalled();
  });

  it("does not subscribe to system changes for a fixed light or dark preference", async () => {
    const addEventListener = vi.fn();
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener,
      removeEventListener: vi.fn(),
    }));
    const fixed = { ...view(OPTIONS), theme: "dark" };
    const { unmount } = await renderPicker(fixed);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(addEventListener).not.toHaveBeenCalled();
    unmount();
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

describe("PickerApp expanded preview and card row height", () => {
  it("renders the card grid with the reduced ~5% row floor", async () => {
    const { container } = await renderPicker(view(OPTIONS));
    const grid = scrollRegion(container).querySelector<HTMLElement>(".grid");
    expect(grid, "the card grid lives in the scroll region").not.toBeNull();
    // 148px floors the row (was 156px); `1fr` lets full-height viewports
    // stretch it back up, so long cards only scroll inside single cards.
    expect(grid?.style.gridAutoRows).toBe("minmax(148px, 1fr)");
  });

  it("keeps the expanded preview and hovered cards free of drop shadows", async () => {
    const { container } = await renderPicker(view(OPTIONS));
    const maximize = container.querySelector<HTMLElement>('button[aria-label^="展开"]');
    expect(maximize, "a ready card offers the expand affordance").not.toBeNull();
    fireEvent.click(maximize!);

    // The preview modal separates itself with its border and the dimmed
    // backdrop, not a shadow layer.
    const preview = container.querySelector<HTMLElement>(
      'div[class*="rounded-2xl"][class*="bg-surface-card"]',
    );
    expect(preview, "the expanded preview mounts a modal").not.toBeNull();
    expect(preview?.className).not.toMatch(/shadow(-\w+)?/);

    // Card buttons may keep only the 1px border ring; any other shadow
    // declaration (the removed hover drop glow, selected/normal states) fails.
    for (const button of scrollRegion(container).querySelectorAll("button")) {
      for (const match of button.className.matchAll(/shadow-\[[^\]]*\]/g)) {
        expect(match[0]).toContain("0_0_0_1px");
      }
    }

    // Close (X) returns to the card grid.
    fireEvent.click(container.querySelector('button[aria-label="关闭预览"]')!);
    expect(container.querySelectorAll('button[aria-label^="展开"]').length).toBeGreaterThan(0);
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

    // The hover lift (-translate-y-0.5 = 2px) must never clip the first row's
    // top border at the scroll container edge, so it reserves top padding.
    expect(region.className).toMatch(/\bpt-(1|2|3)\b/);

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
  it("gives selectable cards a visible hover and keyboard-focus treatment", async () => {
    const { container } = await renderPicker(view(OPTIONS));
    const polish = container.querySelector<HTMLElement>("button[aria-label^='润色']");
    const original = container.querySelector<HTMLElement>("button[aria-label^='原文']");
    expect(polish?.getAttribute("data-picker-card-index")).toBe("1");
    expect(original?.getAttribute("data-picker-card-index")).toBe("0");
    expect(polish?.className).toContain("focus-visible:ring-2");
    expect(polish?.className).not.toContain("hover:bg-");
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
    fireEvent.click(container.querySelector<HTMLElement>("button[aria-label^='润色']") as HTMLElement);
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
    fireEvent.click(container.querySelector<HTMLElement>("button[aria-label^='润色']") as HTMLElement);
    fireEvent.click(container.querySelector<HTMLElement>("button[aria-label^='要点']") as HTMLElement);
    expect(h.selectPickerOption).not.toHaveBeenCalled();
    fireEvent.click(container.querySelector<HTMLElement>("button[aria-label^='原文']") as HTMLElement);
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

describe("PickerApp column policy", () => {
  beforeEach(() => {
    h.reportPickerLayout.mockClear();
  });

  it("applies the native column count when the viewport supports it", async () => {
    // 1344px viewport (built-in/external panel width) supports the native 3.
    vi.stubGlobal("innerWidth", 1344);
    const { container } = await renderPicker(view(OPTIONS, 14, 3));
    const grid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    // Three columns × three cards = a single horizontal row (invariant).
    expect(grid?.style.gridTemplateColumns).toBe("repeat(3, minmax(0, 1fr))");
  });

  it("clamps the native column count to the viewport that actually fits", async () => {
    // Native asked for three, but this window measures 652px: forcing three
    // columns would clip the cards, so the measured width decides the grid.
    // A stale native number must never produce horizontal overflow.
    vi.stubGlobal("innerWidth", 652);
    const { container } = await renderPicker(view(OPTIONS, 15, 3));
    const grid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    expect(grid?.style.gridTemplateColumns).toContain("repeat(2");
  });

  it("widens the grid when the viewport grows after a monitor switch", async () => {
    // Opens on the narrow measure, then the window lands on the wider display:
    // the resize event must re-derive the layout instead of pinning the old one.
    vi.stubGlobal("innerWidth", 652);
    const { container } = await renderPicker(view(OPTIONS, 16, 3));
    const grid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    expect(grid?.style.gridTemplateColumns).toContain("repeat(2");

    vi.stubGlobal("innerWidth", 1344);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    const widened = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    expect(widened?.style.gridTemplateColumns).toBe("repeat(3, minmax(0, 1fr))");
  });

  it("falls back to the width heuristic for payloads without a column count", async () => {
    vi.stubGlobal("innerWidth", 652);
    const { container } = await renderPicker(view(OPTIONS, 17));
    const grid = container.querySelector<HTMLElement>("[data-picker-scroll] > div");
    expect(grid?.style.gridTemplateColumns).toContain("repeat(2");
  });

  it("reports the measured viewport and applied grid to the native log", async () => {
    vi.stubGlobal("innerWidth", 1344);
    vi.stubGlobal("innerHeight", 809);
    await renderPicker(view(OPTIONS, 18, 3));
    expect(h.reportPickerLayout).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: 18,
        viewportWidth: 1344,
        viewportHeight: 809,
        desiredColumns: 3,
        columns: 3,
      }),
    );
  });

  it("reports the divergence when the viewport forces fewer columns", async () => {
    vi.stubGlobal("innerWidth", 652);
    await renderPicker(view(OPTIONS, 19, 3));
    expect(h.reportPickerLayout).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: 19,
        desiredColumns: 3,
        columns: 2,
      }),
    );
  });
});

describe("Picker card hover stability", () => {
  it("adds hover/focus feedback only to selectable cards, not disabled cards", async () => {
    await renderPicker(
      view([
        option({ id: "original", title: "原文", hotkey: "R ⌥", kind: "original" }),
        option({ id: "polish", title: "润色", hotkey: "U" }),
        option({ id: "loading", title: "要点", hotkey: "V", status: "loading", text: null }),
      ]),
    );
    const cards = [
      screen.getByRole("button", { name: "原文（R ⌥）" }),
      screen.getByRole("button", { name: "润色（U）" }),
      screen.getByRole("button", { name: "要点（V）" }),
    ];
    expect(cards[0].className).toContain("hover:brightness-110");
    expect(cards[1].className).toContain("hover:brightness-110");
    expect(cards[0].className).toContain("focus-visible:ring-2");
    expect(cards[2].className).not.toMatch(/hover:/);
    expect(cards[2].className).not.toMatch(/focus-visible:ring/);
    expect(cards[0].getAttribute("data-picker-card-index")).toBe("0");
    expect((cards[0] as HTMLButtonElement).disabled).toBe(false);
    expect((cards[2] as HTMLButtonElement).disabled).toBe(true);
  });

  it("keeps hover feedback on the small controls inside and around cards", async () => {
    await renderPicker(view(OPTIONS));
    const expand = screen.getByRole("button", { name: "展开 润色" });
    expect(expand.className).toContain("hover:");
  });
});
