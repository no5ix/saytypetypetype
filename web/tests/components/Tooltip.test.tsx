import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SectionItem } from "@/settings/layout/PageLayout";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function renderItem() {
  render(<SectionItem title="提示音" tooltip="录音开始时播放" />);
  return screen.getByRole("button", { name: "提示音说明" });
}

describe("Tooltip", () => {
  it("opens on focus, associates its description, and dismisses with Escape or blur", () => {
    const trigger = renderItem();
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    fireEvent.focus(trigger);
    expect(trigger).toHaveAttribute("aria-describedby", screen.getByRole("tooltip").id);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    fireEvent.blur(trigger);
    fireEvent.focus(trigger);
    fireEvent.blur(trigger);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("stays open while the pointer moves from the trigger to the tooltip", () => {
    vi.useFakeTimers();
    const trigger = renderItem();
    fireEvent.mouseEnter(trigger);
    fireEvent.mouseLeave(trigger);
    act(() => vi.advanceTimersByTime(100));
    fireEvent.mouseEnter(screen.getByRole("tooltip"));
    act(() => vi.advanceTimersByTime(200));
    expect(screen.getByRole("tooltip")).toHaveTextContent("录音开始时播放");
    fireEvent.mouseLeave(screen.getByRole("tooltip"));
    act(() => vi.advanceTimersByTime(150));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("avoids the bottom and right edges and dismisses on container scroll", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      return this.tagName === "BUTTON"
        ? ({
            left: window.innerWidth - 24,
            top: window.innerHeight - 30,
            bottom: window.innerHeight - 6,
          } as DOMRect)
        : ({ width: 200, height: 60 } as DOMRect);
    });
    const trigger = renderItem();
    fireEvent.mouseEnter(trigger);
    expect(screen.getByRole("tooltip")).toHaveStyle({
      left: `${window.innerWidth - 208}px`,
      top: `${window.innerHeight - 98}px`,
    });
    fireEvent.scroll(trigger);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("keeps descriptions visible without adding a help button when tooltip is omitted", () => {
    render(<SectionItem title="配置文件路径" description="/config.toml" />);
    expect(screen.getByText("/config.toml")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
