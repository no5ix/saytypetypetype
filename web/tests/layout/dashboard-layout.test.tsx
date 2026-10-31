import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Layout regression tests for the settings window.
 *
 * jsdom performs no layout, so these assert the structural contract the CSS
 * relies on: the sidebar width and non-wrapping labels that keep "Language
 * models" on one line, the update control living on its own row under the
 * product name, and the four cumulative statistic cards sharing a single
 * four-column grid row at every supported window width.
 */

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

const mocks = vi.hoisted(() => ({
  checkForUpdates: vi.fn(),
  getStats: vi.fn(),
}));

vi.mock("@/settings/SettingsProvider", () => ({
  useSettings: () => ({ settings: null, scheduleSave: vi.fn() }),
}));

vi.mock("@/settings/bridge", () => ({
  checkForUpdates: mocks.checkForUpdates,
  downloadUpdate: vi.fn(async () => {}),
  installUpdate: vi.fn(async () => {}),
  onUpdateProgress: () => () => {},
  openHelpGuide: vi.fn(async () => {}),
  getStats: mocks.getStats,
  getHistory: vi.fn(async () => []),
  loadPrompts: vi.fn(async () => []),
  deleteHistory: vi.fn(async () => {}),
  playRecordingPreview: vi.fn(async () => {}),
  stopRecordingPreview: vi.fn(async () => {}),
  onPreviewEnded: () => () => {},
  regenerateHistoryTranscription: vi.fn(async () => {}),
  repolishHistoryEntry: vi.fn(async () => {}),
  retryHistoryTranscription: vi.fn(async () => {}),
}));

beforeEach(() => {
  // The update control keeps its state in a module-level store; a fresh module
  // graph per test keeps one test's update state from leaking into the next.
  vi.resetModules();
  mocks.checkForUpdates.mockReset();
  mocks.getStats.mockReset();
});

afterEach(cleanup);

async function freshSidebar(active: "home" | "llm") {
  const { Sidebar } = await import("@/settings/layout/Sidebar");
  return render(<Sidebar active={active} onNavigate={vi.fn()} />);
}

describe("Sidebar", () => {
  it("is wide enough for one-line English labels and the full product name", async () => {
    mocks.checkForUpdates.mockResolvedValue({ available: false });
    const { container } = await freshSidebar("llm");
    const aside = container.querySelector("aside");
    expect(aside).not.toBeNull();
    // 13rem = 208px: the longest English nav label ("Language models", ~117px)
    // and the product name (~121px) each fit their row with room to spare.
    expect(aside?.className).toContain("w-52");
  });

  it("keeps every navigation label on a single line", async () => {
    mocks.checkForUpdates.mockResolvedValue({ available: false });
    await freshSidebar("llm");
    const labels = [
      "首页",
      "应用设置",
      "系统权限",
      "快捷键",
      "音频模型",
      "大语言模型",
      "热词库",
      "帮助说明",
      "关于",
    ];
    for (const label of labels) {
      expect(screen.getByText(label).className).toContain("whitespace-nowrap");
    }
  });

  it("renders no second row while idle", async () => {
    mocks.checkForUpdates.mockResolvedValue({ available: false });
    await freshSidebar("home");
    expect(screen.queryByRole("button", { name: /更新|重试|安装/ })).toBeNull();
  });

  it("puts the retry control on its own row aligned under the name", async () => {
    mocks.checkForUpdates.mockRejectedValueOnce(new Error("offline"));
    await freshSidebar("home");
    const retry = await screen.findByRole("button", { name: "重试" });
    const name = screen.getByText("SayTypeTypeType");

    // Own row: the button is not a sibling of the icon/name row.
    expect(retry.parentElement).not.toBe(name.parentElement);
    // Header stacks rows vertically.
    expect(name.parentElement?.parentElement?.className).toContain("flex-col");
    // Indented past the icon (28px) plus the row gap (10px) to align with the
    // name, and compact so the header barely grows.
    expect(retry.className).toContain("ml-[38px]");
    expect(retry.className).toContain("px-2");
  });
});

describe("HomePage cumulative statistics", () => {
  const STATS = {
    firstUsedAt: "2026-01-01",
    totalUses: 42,
    totalSessions: 30,
    totalCharacters: 12345,
    dailyCounts: {},
    dailyStats: {},
  };

  async function freshHome() {
    mocks.getStats.mockResolvedValue(STATS);
    const { HomePage } = await import("@/settings/pages/HomePage");
    return render(<HomePage />);
  }

  it("lays all four cumulative cards in one four-column row", async () => {
    await freshHome();
    const labels = ["总使用次数", "总会话次数", "总输入字数", "总节省时间"].map((label) =>
      screen.getByText(label),
    );
    const grids = new Set(labels.map((label) => label.closest(".grid")));
    expect(grids.size).toBe(1);
    const grid = [...grids][0] as HTMLElement;
    // Four columns from 640px upward — below the window's enforced 760px
    // minimum — so the supported window sizes never show a 2x2 grid, while a
    // deliberately narrow dev preview still falls back to two columns.
    expect(grid.className).toContain("grid-cols-2");
    expect(grid.className).toContain("sm:grid-cols-4");
    expect(grid.className).not.toContain("xl:grid-cols-4");
  });

  it("keeps the daily row separate and labels non-wrapping", async () => {
    await freshHome();
    const daily = screen.getByText("今日会话次数").closest(".grid") as HTMLElement;
    const total = screen.getByText("总会话次数").closest(".grid") as HTMLElement;
    expect(daily).not.toBe(total);
    expect(daily.className).toContain("sm:grid-cols-3");
    // Longest English label ("Total sessions") must ellipsize, never wrap or
    // spill out of the ~90px card interior at the minimum window width.
    expect(screen.getByText("总会话次数").className).toContain("truncate");
  });
});
