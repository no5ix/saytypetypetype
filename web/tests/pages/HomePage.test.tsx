import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dayKey } from "@/settings/components/Heatmap";

const mocks = vi.hoisted(() => ({
  getStats: vi.fn(),
  getHistory: vi.fn(),
  loadPrompts: vi.fn(),
  onPreviewEnded: vi.fn(),
  onStatsChanged: vi.fn(),
}));

vi.mock("@/settings/bridge", () => ({
  deleteHistory: vi.fn(),
  getHistory: mocks.getHistory,
  getStats: mocks.getStats,
  loadPrompts: mocks.loadPrompts,
  onPreviewEnded: mocks.onPreviewEnded,
  onStatsChanged: mocks.onStatsChanged,
  playRecordingPreview: vi.fn(),
  regenerateHistoryTranscription: vi.fn(),
  repolishHistoryEntry: vi.fn(),
  retryHistoryTranscription: vi.fn(),
  stopRecordingPreview: vi.fn(),
}));

import { HomePage } from "@/settings/pages/HomePage";

function statsWith(usage: { sessions: number; words: number; recordingMs: number }) {
  return {
    totalUses: 10,
    totalSessions: usage.sessions,
    totalCharacters: 123,
    dailyCounts: {},
    dailyStats: { [dayKey(new Date())]: usage },
  };
}

function dailyCardValue(label: string): string {
  const card = screen.getByText(label).closest("div.flex.min-h-28");
  return card?.querySelector("span")?.textContent || "";
}

let statsListener: ((payload: { reason?: string }) => void) | null = null;

beforeEach(() => {
  statsListener = null;
  mocks.getStats.mockResolvedValue(statsWith({ sessions: 1, words: 20, recordingMs: 30000 }));
  mocks.getHistory.mockResolvedValue([]);
  mocks.loadPrompts.mockResolvedValue([]);
  mocks.onPreviewEnded.mockReturnValue(() => {});
  mocks.onStatsChanged.mockImplementation((cb: (p: { reason?: string }) => void) => {
    statsListener = cb;
    return () => {
      statsListener = null;
    };
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("HomePage dashboard live refresh", () => {
  it("shows today's sessions, words and mic time from the backend", async () => {
    render(<HomePage />);
    await waitFor(() => expect(mocks.getStats).toHaveBeenCalled());
    expect(dailyCardValue("今日会话次数")).toBe("1");
    expect(dailyCardValue("今日输入字数")).toBe("20");
    expect(dailyCardValue("今日录音时长")).toBe("30秒");
  });

  it("updates all three daily cards when the backend reports a delivered session", async () => {
    render(<HomePage />);
    await waitFor(() => expect(mocks.getStats).toHaveBeenCalledTimes(1));
    const listener = statsListener;
    if (!listener) throw new Error("stats listener was not registered");

    // The backend persisted the delivered session; a fresh get_stats now
    // returns the updated counters. No navigation or restart in between.
    mocks.getStats.mockResolvedValue(statsWith({ sessions: 2, words: 42, recordingMs: 61000 }));
    listener({ reason: "session-delivered" });

    await waitFor(() => expect(mocks.getStats).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(dailyCardValue("今日会话次数")).toBe("2"));
    expect(dailyCardValue("今日输入字数")).toBe("42");
    expect(dailyCardValue("今日录音时长")).toBe("1分钟");
    // The history list reloads alongside the counters so the new entry appears.
    await waitFor(() => expect(mocks.getHistory).toHaveBeenCalledTimes(2));
  });

  it("coalesces bursty stats events into a single reload", async () => {
    render(<HomePage />);
    await waitFor(() => expect(mocks.getStats).toHaveBeenCalledTimes(1));

    const listener = statsListener;
    if (!listener) throw new Error("stats listener was not registered");
    listener({ reason: "session-delivered" });
    listener({ reason: "session-delivered" });
    listener({ reason: "session-delivered" });

    await waitFor(() => expect(mocks.getStats).toHaveBeenCalled());
    // Debounce collapses the burst: exactly one extra load, not three.
    await new Promise((r) => setTimeout(r, 300));
    expect(mocks.getStats).toHaveBeenCalledTimes(2);
  });

  it("unsubscribes the stats listener on unmount", async () => {
    const view = render(<HomePage />);
    await waitFor(() => expect(statsListener).toBeInstanceOf(Function));
    view.unmount();
    expect(statsListener).toBeNull();
  });
});
