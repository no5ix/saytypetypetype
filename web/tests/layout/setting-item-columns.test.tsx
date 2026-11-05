// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Layout regression tests for `SectionItem` label/input columns.
 *
 * jsdom performs no layout, so these assert the structural contract the CSS
 * relies on: every API row on the Language models page must hand its control
 * the same measured label column, so the URL input shares the exact left edge,
 * right edge, and width of the API Key and Model ID inputs.
 *
 * The bug these tests pin down: the label column used to be content-sized, and
 * the control column (`flex-1`) has a zero flex-basis — zero basis means zero
 * shrink weight, so a long `description` made the label absorb all the shrink
 * and collapsed the URL input to its intrinsic minimum width.
 */

const mocks = vi.hoisted(() => ({
  settings: {
    parsedConfig: {
      llm: { provider: "openai_compatible" } as Record<string, unknown>,
    },
  },
  save: vi.fn(),
  loadPrompts: vi.fn(),
  savePrompts: vi.fn(),
  loadBuiltinPrompts: vi.fn(),
}));

vi.mock("@/settings/SettingsProvider", () => ({
  useSettings: () => ({ settings: mocks.settings, scheduleSave: mocks.save }),
}));
vi.mock("@/settings/bridge", () => ({
  loadPrompts: mocks.loadPrompts,
  savePrompts: mocks.savePrompts,
  loadBuiltinPrompts: mocks.loadBuiltinPrompts,
}));

import { SETTING_LABEL_COLUMN, SectionItem } from "@/settings/layout/PageLayout";
import { LLMPage } from "@/settings/pages/LLMPage";

beforeEach(() => {
  mocks.settings.parsedConfig.llm = {
    provider: "openai_compatible",
    openai_compatible: {
      url: "http://127.0.0.1:8000/v1",
      api_key: "sk-local",
      model: "qwen3-asr-1.7b",
    },
  };
  mocks.save.mockResolvedValue(undefined);
  mocks.loadPrompts.mockResolvedValue([]);
  mocks.loadBuiltinPrompts.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/** `[labelColumn, controlColumn]` for the `SectionItem` that owns `input`. */
function columnsOf(input: HTMLElement): [HTMLElement, HTMLElement] {
  const row = input.closest(".group");
  if (!(row instanceof HTMLElement)) throw new Error("input is not inside a SectionItem row");
  const label = row.firstElementChild?.firstElementChild;
  const control = label?.nextElementSibling;
  if (!(label instanceof HTMLElement) || !(control instanceof HTMLElement)) {
    throw new Error("SectionItem row is missing its label/control columns");
  }
  return [label, control];
}

/**
 * Column classes that decide width, in sorted order.
 *
 * Cross-axis classes (`mb-auto`, `self-*`, `items-*`, `justify-*`) only position
 * a control vertically inside a taller row — a row with a `description` aligns
 * its input with the title — so they are excluded from the width comparison.
 */
function widthClasses(el: HTMLElement): string {
  const ignored = /^mb-|^mt-|^self-|^items-|^justify-|^text-/;
  return el.className
    .split(/\s+/)
    .filter((cls) => cls && !ignored.test(cls))
    .sort()
    .join(" ");
}

describe("SETTING_LABEL_COLUMN token", () => {
  it("is a responsive share of the row, never a hardcoded pixel width", () => {
    expect(SETTING_LABEL_COLUMN).toMatch(/^w-\[\d+(\.\d+)?%\]$/);
    expect(SETTING_LABEL_COLUMN).not.toMatch(/px/);
  });
});

describe("Language models API rows share one input column", () => {
  it("URL, API Key and Model ID controls sit in identical columns", async () => {
    render(<LLMPage />);
    const url = (await screen.findByPlaceholderText("http://127.0.0.1:8000/v1")) as HTMLElement;
    const apiKey = screen.getByPlaceholderText("sk-...") as HTMLElement;
    const modelId = screen.getByDisplayValue("qwen3-asr-1.7b") as HTMLElement;

    const rows = [url, apiKey, modelId].map(columnsOf);
    const labels = rows.map(([label]) => label);
    const controls = rows.map(([, control]) => control);

    // Identical width-defining columns in all three rows => identical left
    // edge, right edge and width for the three inputs.
    expect(new Set(labels.map(widthClasses)).size).toBe(1);
    expect(new Set(controls.map(widthClasses)).size).toBe(1);

    // The label column is measured (fixed share, cannot be squeezed by content)
    // and the control column can fill it (min-w-0 beats the intrinsic width).
    for (const label of labels) {
      expect(label.className).toContain(SETTING_LABEL_COLUMN);
      expect(label.className).toMatch(/\bshrink-0\b/);
    }
    for (const control of controls) {
      expect(control.className).toMatch(/\bmin-w-0\b/);
      expect(control.className).toMatch(/\bflex-1\b/);
      expect(control.className).not.toMatch(/max-w/);
    }
  });

  it("keeps the URL helper text in the label column, so it cannot shrink the input", async () => {
    render(<LLMPage />);
    const url = (await screen.findByPlaceholderText("http://127.0.0.1:8000/v1")) as HTMLElement;
    const [label, control] = columnsOf(url);

    const helper = screen.getByText(
      "填写 API 基础地址（如 http://127.0.0.1:8000/v1），应用会自动拼接 /chat/completions；旧配置中的完整地址依然可用。",
    );
    expect(label).toContainElement(helper);
    expect(control).not.toContainElement(helper);
    // Long URLs must wrap inside the narrower label column instead of widening it.
    expect(helper.className).toMatch(/\b(break-words|break-all)\b/);
  });

  it("keeps existing URL values editable through the same controls", async () => {
    render(<LLMPage />);
    const url = (await screen.findByPlaceholderText(
      "http://127.0.0.1:8000/v1",
    )) as HTMLInputElement;
    expect(url).toBeEnabled();
    expect(url).toHaveValue("http://127.0.0.1:8000/v1");
    expect(url).not.toHaveAttribute("readonly");
    expect(url).not.toHaveAttribute("disabled");
  });

  it("leaves rows that do not opt into the label column untouched", async () => {
    mocks.loadPrompts.mockResolvedValue([
      { id: "p1", title: "Polish", hotkey: "", hotkey_mode: "toggle", prompt: "be brief" },
    ]);
    render(<LLMPage />);
    const promptTitle = (await screen.findByPlaceholderText("模板名称")) as HTMLElement;
    const [label, control] = columnsOf(promptTitle);

    // Default geometry is preserved for every other settings page/row.
    expect(label.className).not.toContain("shrink-0");
    expect(label.className).not.toContain(SETTING_LABEL_COLUMN);
    expect(control.className).toContain("max-w-3/4");
  });
});

describe("SectionItem column contract", () => {
  it("opting into a label column replaces the content-sized label and max-w clamp", () => {
    const { container } = render(
      <SectionItem
        title="URL"
        labelClassName={SETTING_LABEL_COLUMN}
        description="a long helper sentence that used to push the control out of the row"
        action={<input placeholder="column-input" />}
      />,
    );
    const input = container.querySelector("input") as HTMLElement;
    const [label, control] = columnsOf(input);
    expect(label.className).toContain(SETTING_LABEL_COLUMN);
    expect(control.className).toMatch(/\bmin-w-0\b/);
    expect(control.className).not.toMatch(/max-w/);
  });

  it("keeps the legacy content-sized label row when no column is requested", () => {
    const { container } = render(
      <SectionItem title="API Key" action={<input placeholder="legacy-input" />} />,
    );
    const input = container.querySelector("input") as HTMLElement;
    const [label, control] = columnsOf(input);
    expect(label.className).toBe("min-w-0");
    expect(control.className).toContain("max-w-3/4");
  });
});
