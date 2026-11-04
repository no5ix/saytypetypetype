import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  settings: {
    parsedConfig: {
      llm: { provider: "deepseek" } as Record<string, unknown>,
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

import { LLMPage } from "@/settings/pages/LLMPage";

beforeEach(() => {
  mocks.settings.parsedConfig.llm = { provider: "deepseek" };
  mocks.save.mockResolvedValue(undefined);
  mocks.loadPrompts.mockResolvedValue([]);
  mocks.loadBuiltinPrompts.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("LLMPage URL field (base URL semantics)", () => {
  it("presets are base URLs — no full /chat/completions endpoint in the UI", async () => {
    render(<LLMPage />);
    await waitFor(() => expect(mocks.loadPrompts).toHaveBeenCalled());
    const urlInput = screen.getByPlaceholderText("http://127.0.0.1:8000/v1");
    expect(urlInput).toHaveValue("https://api.deepseek.com/v1");
    // The visible helper text explains that the app appends the path itself.
    expect(
      screen.getByText(
        "填写 API 基础地址（如 http://127.0.0.1:8000/v1），应用会自动拼接 /chat/completions；旧配置中的完整地址依然可用。",
      ),
    ).toBeInTheDocument();
  });

  it("a legacy full endpoint saved by an older version is preserved verbatim", async () => {
    mocks.settings.parsedConfig.llm = {
      provider: "openai_compatible",
      openai_compatible: {
        url: "http://127.0.0.1:8000/v1/chat/completions",
        api_key: "sk-old",
        model: "local",
      },
    };
    render(<LLMPage />);
    await waitFor(() => expect(mocks.loadPrompts).toHaveBeenCalled());
    expect(screen.getByPlaceholderText("http://127.0.0.1:8000/v1")).toHaveValue(
      "http://127.0.0.1:8000/v1/chat/completions",
    );
  });

  it("typing a base URL persists exactly what the user entered", async () => {
    render(<LLMPage />);
    const urlInput = (await screen.findByPlaceholderText(
      "http://127.0.0.1:8000/v1",
    )) as HTMLInputElement;
    fireEvent.change(urlInput, { target: { value: "http://127.0.0.1:8000/v1" } });
    fireEvent.blur(urlInput);
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        llm: {
          provider: "deepseek",
          deepseek: { url: "http://127.0.0.1:8000/v1" },
        },
      }),
    );
  });
});
