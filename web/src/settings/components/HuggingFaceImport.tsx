import { AlertCircle, CircleCheck, Download, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useT } from "@/i18n";
import {
  cancelHfModelImport,
  type HfImportResult,
  importHfModel,
  type ModelDownloadProgress,
  onModelDownloadProgress,
} from "@/settings/bridge";
import { Button } from "@/settings/components/Button";
import { Input } from "@/settings/components/Input";
import { Section, SectionContent, SectionHeader } from "@/settings/layout/PageLayout";

/** Backend sentinel for a user-cancelled import; never shown to the user. */
const CANCELLED = "导入已取消";

type ImportState =
  | { kind: "idle" }
  | { kind: "importing" }
  | { kind: "cancelling" }
  | { kind: "success"; text: string }
  | { kind: "error"; text: string };

interface ImportProgress {
  percent: number;
  detail?: string;
}

/**
 * Import an ASR model from a public Hugging Face repository.
 *
 * The backend only accepts repositories that already publish the sherpa-ONNX
 * NeMo transducer file set; anything else comes back as a readable error. This
 * form intentionally stays small: one repository ID, one button, progress and
 * clear feedback — model management lives in the cards below.
 */
export function HuggingFaceImport({ onImported }: { onImported: () => void | Promise<void> }) {
  const t = useT();
  const [repoId, setRepoId] = useState("");
  const [state, setState] = useState<ImportState>({ kind: "idle" });
  const [progress, setProgress] = useState<ImportProgress | null>(null);

  const busy = state.kind === "importing" || state.kind === "cancelling";

  useEffect(() => {
    if (!busy) return;
    return onModelDownloadProgress((event: ModelDownloadProgress) => {
      // Imported models use the `hf-` id prefix; other downloads keep their own
      // card-level progress and must not leak into this panel.
      if (!event.model_id.startsWith("hf-")) return;
      if (event.status !== "downloading" && event.status !== "completed") return;
      setProgress({
        percent:
          typeof event.progress === "number"
            ? Math.max(0, Math.min(100, Math.round(event.progress)))
            : 0,
        detail: event.message,
      });
    });
  }, [busy]);

  const submit = useCallback(async () => {
    const id = repoId.trim();
    if (!id || busy) return;
    setState({ kind: "importing" });
    setProgress({ percent: 0 });
    try {
      const result: HfImportResult = await importHfModel(id);
      setRepoId("");
      setState({
        kind: "success",
        text: result.already_imported
          ? t("{name} 已经导入，无需重复下载", { name: result.name })
          : t("{name} 已导入，可在下方模型列表中选择", { name: result.name }),
      });
      await onImported();
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      setState(
        text === CANCELLED ? { kind: "idle" } : { kind: "error", text: text || t("导入失败") },
      );
    } finally {
      setProgress(null);
    }
  }, [busy, onImported, repoId, t]);

  const cancel = useCallback(async () => {
    if (state.kind !== "importing") return;
    setState({ kind: "cancelling" });
    try {
      await cancelHfModelImport();
    } catch {
      /* the import reports its own outcome; a failed cancel click is harmless */
    }
  }, [state.kind]);

  return (
    <Section>
      <SectionHeader
        title={t("从 Hugging Face 导入")}
        subtitle={t("输入公开仓库 ID，下载并注册为本地模型")}
      />
      <SectionContent>
        <div className="flex items-center gap-2">
          <Input
            value={repoId}
            onChange={setRepoId}
            placeholder={t("owner/model，例如 nvidia/parakeet-tdt-0.6b-v2")}
            aria-label={t("Hugging Face 仓库 ID")}
            disabled={busy}
            className="flex-1"
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void submit();
              }
            }}
          />
          {state.kind === "importing" ? (
            <Button variant="ghost" onClick={() => void cancel()}>
              <X size={16} />
              {t("停止")}
            </Button>
          ) : (
            <Button
              variant="accent"
              onClick={() => void submit()}
              disabled={busy || repoId.trim().length === 0}
            >
              <Download size={16} />
              {state.kind === "cancelling" ? t("正在停止…") : t("下载并导入")}
            </Button>
          )}
        </div>

        <p className="text-xs text-text-muted leading-relaxed">
          {t(
            "仅支持 sherpa-ONNX 格式的公开仓库（encoder / decoder / joiner .onnx 与 tokens.txt）。NeMo 检查点、MLX 或 Transformers 权重需要先转换为 ONNX。",
          )}
        </p>

        {busy && (
          <div className="space-y-1.5 pt-1">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-fill-track">
              <div
                className="h-full rounded-full bg-accent transition-[width]"
                style={{ width: `${progress?.percent ?? 0}%` }}
              />
            </div>
            <p className="truncate text-[11px] text-text-muted" aria-live="polite">
              {progress?.detail
                ? t("{detail}（{percent}%）", {
                    detail: progress.detail,
                    percent: progress.percent,
                  })
                : t("正在下载并准备模型…（{percent}%）", { percent: progress?.percent ?? 0 })}
            </p>
          </div>
        )}

        {state.kind === "success" && (
          <p className="flex items-start gap-1.5 text-xs text-success" aria-live="polite">
            <CircleCheck className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">{state.text}</span>
          </p>
        )}

        {state.kind === "error" && (
          <p className="flex items-start gap-1.5 text-xs text-error" aria-live="polite">
            <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">{state.text}</span>
          </p>
        )}
      </SectionContent>
    </Section>
  );
}
