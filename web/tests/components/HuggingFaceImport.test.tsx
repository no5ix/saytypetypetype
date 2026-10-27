import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
  listeners: new Set<(event: unknown) => void>(),
  importHfModel: vi.fn(),
  cancelHfModelImport: vi.fn(async () => {}),
}));

vi.mock("@/settings/bridge", () => ({
  importHfModel: h.importHfModel,
  cancelHfModelImport: h.cancelHfModelImport,
  onModelDownloadProgress: (listener: (event: unknown) => void) => {
    h.listeners.add(listener);
    return () => h.listeners.delete(listener);
  },
}));

import type { ModelDownloadProgress } from "@/settings/bridge";
import { HuggingFaceImport } from "@/settings/components/HuggingFaceImport";

function emitProgress(event: ModelDownloadProgress) {
  act(() => {
    for (const listener of h.listeners) listener(event);
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const SUCCESS = {
  model_id: "hf-nvidia-parakeet-tdt-0-6b-v2-1a2b3c4d",
  name: "Parakeet TDT 0.6B v2",
  already_imported: false,
};

function renderImport(onImported = vi.fn()) {
  const result = render(<HuggingFaceImport onImported={onImported} />);
  const input = screen.getByLabelText("Hugging Face 仓库 ID") as HTMLInputElement;
  return { ...result, input, onImported };
}

beforeEach(() => {
  h.listeners.clear();
  h.importHfModel.mockReset();
  h.cancelHfModelImport.mockClear();
});

afterEach(() => {
  cleanup();
});

describe("HuggingFaceImport", () => {
  it("keeps the download button disabled until a repository ID is entered", () => {
    renderImport();
    const button = screen.getByRole("button", { name: /下载并导入/ }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Hugging Face 仓库 ID"), {
      target: { value: "nvidia/parakeet-tdt-0.6b-v2" },
    });
    expect(button.disabled).toBe(false);
  });

  it("imports a repository and reports success", async () => {
    h.importHfModel.mockResolvedValue(SUCCESS);
    const { input, onImported } = renderImport();

    fireEvent.change(input, { target: { value: "  nvidia/parakeet-tdt-0.6b-v2  " } });
    fireEvent.click(screen.getByRole("button", { name: /下载并导入/ }));

    await waitFor(() =>
      expect(h.importHfModel).toHaveBeenCalledWith("nvidia/parakeet-tdt-0.6b-v2"),
    );
    expect(await screen.findByText(/Parakeet TDT 0.6B v2 已导入/)).toBeTruthy();
    expect(onImported).toHaveBeenCalledTimes(1);
    expect(input.value).toBe("");
  });

  it("explains that an already imported model needs no second download", async () => {
    h.importHfModel.mockResolvedValue({ ...SUCCESS, already_imported: true });
    const { input } = renderImport();

    fireEvent.change(input, { target: { value: "nvidia/parakeet-tdt-0.6b-v2" } });
    fireEvent.click(screen.getByRole("button", { name: /下载并导入/ }));

    expect(await screen.findByText(/已经导入，无需重复下载/)).toBeTruthy();
  });

  it("shows the backend error for unsupported repositories", async () => {
    h.importHfModel.mockRejectedValue(
      new Error(
        "无法导入：该仓库缺少 sherpa-ONNX 需要的 encoder .onnx 文件。仓库中的文件格式为：NeMo 检查点（.nemo）。",
      ),
    );
    const { input, onImported } = renderImport();

    fireEvent.change(input, { target: { value: "nvidia/parakeet-tdt-0.6b-v2" } });
    fireEvent.click(screen.getByRole("button", { name: /下载并导入/ }));

    expect(await screen.findByText(/缺少 sherpa-ONNX 需要/)).toBeTruthy();
    expect(onImported).not.toHaveBeenCalled();
    // The input keeps the repository ID so the user can correct a typo.
    expect(input.value).toBe("nvidia/parakeet-tdt-0.6b-v2");
  });

  it("reports download progress for the imported model only", async () => {
    const pending = deferred<typeof SUCCESS>();
    h.importHfModel.mockReturnValue(pending.promise);
    const { input } = renderImport();

    fireEvent.change(input, { target: { value: "org/model" } });
    fireEvent.click(screen.getByRole("button", { name: /下载并导入/ }));

    await waitFor(() => expect(h.listeners.size).toBe(1));
    emitProgress({
      model_id: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8",
      status: "downloading",
      progress: 90,
    });
    expect(screen.queryByText(/90%/)).toBeNull();

    emitProgress({
      model_id: "hf-org-model-1a2b3c4d",
      status: "downloading",
      progress: 42,
      message: "下载 encoder.int8.onnx",
    });
    expect(await screen.findByText(/下载 encoder.int8.onnx（42%）/)).toBeTruthy();

    await act(async () => {
      pending.resolve(SUCCESS);
    });
  });

  it("cancels a running import", async () => {
    const pending = deferred<typeof SUCCESS>();
    h.importHfModel.mockReturnValue(pending.promise);
    const { input } = renderImport();

    fireEvent.change(input, { target: { value: "org/model" } });
    fireEvent.click(screen.getByRole("button", { name: /下载并导入/ }));

    const stop = await screen.findByRole("button", { name: /停止/ });
    fireEvent.click(stop);
    await waitFor(() => expect(h.cancelHfModelImport).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: /正在停止/ })).toBeTruthy();

    // The backend confirms cancellation with a plain error; the form returns to
    // idle instead of showing a failure.
    await act(async () => {
      pending.reject(new Error("导入已取消"));
    });
    await waitFor(() => expect(screen.queryByText(/导入已取消/)).toBeNull());
    expect(screen.getByRole("button", { name: /下载并导入/ })).toBeTruthy();
  });

  it("submits with Enter", async () => {
    h.importHfModel.mockResolvedValue(SUCCESS);
    const { input } = renderImport();

    fireEvent.change(input, { target: { value: "org/model" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => expect(h.importHfModel).toHaveBeenCalledWith("org/model"));
  });
});
