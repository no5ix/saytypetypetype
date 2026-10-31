import { Copy, Loader2, Pause, Play, RefreshCw, RotateCw, Sparkles, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import {
  deleteHistory,
  getHistory,
  getStats,
  loadPrompts,
  onPreviewEnded,
  playRecordingPreview,
  regenerateHistoryTranscription,
  repolishHistoryEntry,
  retryHistoryTranscription,
  stopRecordingPreview,
} from "@/settings/bridge";
import { Button } from "@/settings/components/Button";
import { dayKey, formatRecording, Heatmap } from "@/settings/components/Heatmap";
import { PromptMenu } from "@/settings/components/PromptMenu";
import { Toast, useToast } from "@/settings/components/Toast";
import {
  PageHeader,
  PageLayout,
  Section,
  SectionContent,
  SectionHeader,
  SectionItemList,
} from "@/settings/layout/PageLayout";
import { formatCompact } from "@/settings/lib/format";
import type { PromptItem } from "@/settings/types/prompts";
import type { DailyUsage, Stats } from "@/settings/types/stats";

/* ---------- pure helpers ---------- */

function greetingKey(): string {
  const h = new Date().getHours();
  if (h < 6) return "夜深了";
  if (h < 11) return "早上好";
  if (h < 13) return "中午好";
  if (h < 18) return "下午好";
  return "晚上好";
}

const WEEKDAYS_EN = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** Localised label for a `YYYY-MM-DD` history date. */
function dateLabel(key: string, today: string, yesterday: string, en: boolean): string {
  if (key === today) return en ? "Today" : "今天";
  if (key === yesterday) return en ? "Yesterday" : "昨天";
  const d = new Date(`${key}T00:00:00`);
  return en
    ? `${WEEKDAYS_EN[d.getDay()]}, ${d.getMonth() + 1}/${d.getDate()}`
    : `${d.getMonth() + 1}月${d.getDate()}日 ${["周日", "周一", "周二", "周三", "周四", "周五", "周六"][d.getDay()]}`;
}

interface HistoryItem {
  ts: string;
  text: string;
  status?: "success" | "failed";
  audioPath?: string;
  error?: string;
  retryOf?: string;
  /** Prompt template this entry was polished with; absent when never polished. */
  promptId?: string;
}

/* ---------- component ---------- */

export function HomePage() {
  const { language, t } = useI18n();
  const [stats, setStats] = useState<Stats | null>(null);
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [days, setDays] = useState(1);
  const [retryingTs, setRetryingTs] = useState<string | null>(null);
  const [regeneratingTs, setRegeneratingTs] = useState<string | null>(null);
  const [repolishingTs, setRepolishingTs] = useState<string | null>(null);
  // The entry whose recording is playing. Only one plays at a time, so this
  // doubles as the play/pause state for every row's button.
  const [playingTs, setPlayingTs] = useState<string | null>(null);
  const { toast, showToast } = useToast();
  // Why the last re-polish failed, keyed by entry. Shown inline under the row:
  // the message names what to fix (model, key, quota), so it has to be readable
  // rather than a transient toast.
  const [repolishErrors, setRepolishErrors] = useState<Record<string, string>>({});
  // Template picker for polishing: which row it is open on, and its button.
  const [polishMenu, setPolishMenu] = useState<{ ts: string; anchor: HTMLElement } | null>(null);
  const [prompts, setPrompts] = useState<PromptItem[]>([]);
  const closePolishMenu = useCallback(() => setPolishMenu(null), []);

  // Playback ends on its own, or because another row replaced it; either way the
  // backend reports it, so the button never stays stuck on pause.
  useEffect(() => onPreviewEnded(() => setPlayingTs(null)), []);

  const load = useCallback(async () => {
    try {
      setStats((await getStats()) as unknown as Stats);
    } catch {
      /* */
    }
  }, []);

  const loadHist = useCallback(async (d: number) => {
    try {
      setHistory(((await getHistory(d)) || []) as HistoryItem[]);
    } catch {
      /* */
    }
  }, []);

  useEffect(() => {
    load();
    loadHist(1);
  }, [load, loadHist]);

  // Reloaded each time the picker opens so templates edited on the LLM page show up.
  const openPolishMenu = useCallback(async (ts: string, anchor: HTMLElement) => {
    try {
      setPrompts(((await loadPrompts()) || []) as PromptItem[]);
    } catch {
      setPrompts([]);
    }
    setPolishMenu({ ts, anchor });
  }, []);

  const repolish = useCallback(
    async (ts: string, promptId: string) => {
      setPolishMenu(null);
      setRepolishingTs(ts);
      setRepolishErrors((prev) => {
        const { [ts]: _dropped, ...rest } = prev;
        return rest;
      });
      try {
        const result = (await repolishHistoryEntry(ts, promptId)) as { text?: string };
        if (result.text) {
          const text = result.text;
          setHistory((prev) =>
            prev.map((entry) => (entry.ts === ts ? { ...entry, text, promptId } : entry)),
          );
        }
        showToast(t("润色完成"));
      } catch (error) {
        setRepolishErrors((prev) => ({ ...prev, [ts]: String(error) }));
        showToast(t("润色失败"), "error");
      } finally {
        setRepolishingTs(null);
      }
    },
    [showToast, t],
  );

  /* —— cards —— */
  const today = new Date();
  const tKey = dayKey(today);
  const y = new Date(today);
  y.setDate(y.getDate() - 1);
  const yKey = dayKey(y);

  const todayUsage: DailyUsage = stats?.dailyStats?.[tKey] ?? {
    sessions: 0,
    words: 0,
    recordingMs: 0,
  };
  const charTotal = stats?.totalCharacters || 0;
  const saved = formatDurationParts(Math.round(charTotal * 0.67));

  // Today first: what the user actually did in this app today, measured from
  // stored per-day usage. Microphone time excludes ASR/LLM/picker latency.
  const dailyCards = [
    { v: formatCompact(todayUsage.sessions), u: t("次"), label: t("今日会话次数") },
    { v: formatCompact(todayUsage.words), u: t("字"), label: t("今日输入字数") },
    {
      v: formatRecording(todayUsage.recordingMs, t),
      u: "",
      label: t("今日录音时长"),
    },
  ];
  const totalCards = [
    { v: formatCompact(stats?.totalUses || 0), u: t("次"), label: t("总使用次数") },
    { v: formatCompact(stats?.totalSessions || 0), u: t("次"), label: t("总会话次数") },
    { v: formatCompact(charTotal), u: t("字"), label: t("总输入字数") },
    { v: saved.value, u: t(saved.unit), label: t("总节省时间") },
  ];

  /* ====== render ====== */
  return (
    <PageLayout>
      <PageHeader title={t(greetingKey())} />

      {/* Today */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        {dailyCards.map((c) => (
          <StatCard key={c.label} value={c.v} unit={c.u} label={c.label} />
        ))}
      </div>

      {/* All-time */}
      <div className="grid grid-cols-2 xl:grid-cols-4 gap-3">
        {totalCards.map((c) => (
          <StatCard key={c.label} value={c.v} unit={c.u} label={c.label} />
        ))}
      </div>

      {/* Heatmap */}
      <Heatmap dailyCounts={stats?.dailyCounts || {}} dailyStats={stats?.dailyStats} />

      {/* History */}
      <Section>
        <SectionHeader title={t("输入记录")} />
        <SectionContent className="p-0!">
          <SectionItemList className="space-y-0">
            {history.length === 0 ? (
              <div className="flex justify-center items-center gap-3 p-4 min-h-10 text-xs text-text-muted">
                {t("暂无输入记录")}
              </div>
            ) : (
              <>
                {(() => {
                  let last = "";
                  return history.map((item) => {
                    const d = new Date(item.ts);
                    const dk = dayKey(d);
                    const show = dk !== last;
                    last = dk;
                    const time = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
                    const failed = item.status === "failed";
                    const retrying = retryingTs === item.ts;
                    const regenerating = regeneratingTs === item.ts;
                    const repolishing = repolishingTs === item.ts;
                    const repolishError = repolishErrors[item.ts];
                    const menuOpen = polishMenu?.ts === item.ts;
                    const playing = playingTs === item.ts;
                    const displayText = failed
                      ? t("转写失败：{error}", {
                          error: item.error || item.text || t("请检查网络连接"),
                        })
                      : item.text;
                    return (
                      <div key={item.ts} className="text-sm">
                        {show && (
                          <div className="flex items-center gap-2 p-2">
                            <div className="flex-1 h-px bg-border-subtle" />
                            <span className="text-text-muted">
                              {dateLabel(dk, tKey, yKey, language === "en")}
                            </span>
                            <div className="flex-1 h-px bg-border-subtle" />
                          </div>
                        )}
                        <div
                          className={`group min-h-8 transition-colors hover:bg-fill-hover relative ${failed ? "bg-warning/10" : ""}`}
                        >
                          <div className="px-4 flex items-center gap-2 min-h-8 ">
                            <span className="text-xs text-text-muted shrink-0">{time}</span>
                            <div className="flex-1 min-w-0">
                              <p
                                className={`truncate leading-[1.4] ${failed ? "text-warning" : "text-text-dim"}`}
                              >
                                {displayText}
                              </p>
                              {repolishError && (
                                <p className="pb-1 text-xs leading-[1.4] text-warning">
                                  {t("润色失败：{error}", { error: repolishError })}
                                </p>
                              )}
                            </div>
                          </div>
                          <div
                            className={`flex items-center gap-2 transition-opacity duration-200 absolute top-1/2 right-2 -translate-y-1/2 h-full ${
                              retrying || regenerating || repolishing || menuOpen
                                ? "opacity-100"
                                : "opacity-0 group-hover:opacity-100"
                            }`}
                          >
                            {item.audioPath && (
                              <Button
                                size="icon"
                                variant="accent"
                                title={playing ? t("暂停") : t("播放")}
                                onClick={async () => {
                                  try {
                                    if (playing) {
                                      await stopRecordingPreview();
                                      setPlayingTs(null);
                                      return;
                                    }
                                    // Starting a preview stops whatever else was
                                    // playing, so claim the state right away.
                                    setPlayingTs(item.ts);
                                    await playRecordingPreview(item.audioPath || "");
                                  } catch {
                                    setPlayingTs(null);
                                  }
                                }}
                              >
                                {playing ? <Pause size={14} /> : <Play size={14} />}
                              </Button>
                            )}
                            {failed ? (
                              <Button
                                size="icon"
                                variant="accent"
                                disabled={retrying}
                                onClick={async () => {
                                  setRetryingTs(item.ts);
                                  try {
                                    const result = (await retryHistoryTranscription(item.ts)) as {
                                      text?: string;
                                    };
                                    if (result.text) {
                                      setHistory((prev) =>
                                        prev.map((entry) =>
                                          entry.ts === item.ts
                                            ? {
                                                ...entry,
                                                text: result.text || "",
                                                status: "success",
                                                error: undefined,
                                                retryOf: undefined,
                                              }
                                            : entry,
                                        ),
                                      );
                                    }
                                    await load();
                                    showToast(t("重新转写完成"));
                                  } catch (error) {
                                    showToast(
                                      t("重新转写失败：{error}", { error: String(error) }),
                                      "error",
                                    );
                                  } finally {
                                    setRetryingTs(null);
                                  }
                                }}
                              >
                                <RefreshCw size={14} className={retrying ? "animate-spin" : ""} />
                              </Button>
                            ) : (
                              <>
                                {/* Regenerate: only available when this session kept
                                    its recording (keep-recording was on), so there is
                                    audio to re-transcribe. Updates the record in place
                                    without pasting. */}
                                {item.audioPath && (
                                  <Button
                                    size="icon"
                                    variant="accent"
                                    title={t("重新生成")}
                                    disabled={regenerating}
                                    onClick={async () => {
                                      setRegeneratingTs(item.ts);
                                      try {
                                        const result = (await regenerateHistoryTranscription(
                                          item.ts,
                                        )) as { text?: string };
                                        if (result.text) {
                                          setHistory((prev) =>
                                            prev.map((entry) =>
                                              entry.ts === item.ts
                                                ? { ...entry, text: result.text || "" }
                                                : entry,
                                            ),
                                          );
                                        }
                                        showToast(t("重新生成完成"));
                                      } catch (error) {
                                        showToast(
                                          t("重新生成失败：{error}", { error: String(error) }),
                                          "error",
                                        );
                                      } finally {
                                        setRegeneratingTs(null);
                                      }
                                    }}
                                  >
                                    <RotateCw
                                      size={14}
                                      className={regenerating ? "animate-spin" : ""}
                                    />
                                  </Button>
                                )}
                                {/* Polish: on every successful entry, so text
                                    delivered without polishing (or polished
                                    badly) gets a second chance. Opens a template
                                    picker; rewrites the record in place. */}
                                <Button
                                  size="icon"
                                  variant="accent"
                                  title={item.promptId ? t("重新润色") : t("润色")}
                                  disabled={repolishing}
                                  onClick={(event) => {
                                    if (menuOpen) {
                                      setPolishMenu(null);
                                      return;
                                    }
                                    openPolishMenu(item.ts, event.currentTarget);
                                  }}
                                >
                                  {repolishing ? (
                                    <Loader2 size={14} className="animate-spin" />
                                  ) : (
                                    <Sparkles size={14} />
                                  )}
                                </Button>
                                {menuOpen && polishMenu && (
                                  <PromptMenu
                                    anchor={polishMenu.anchor}
                                    prompts={prompts}
                                    currentId={item.promptId}
                                    onSelect={(prompt) => repolish(item.ts, prompt.id)}
                                    onClose={closePolishMenu}
                                  />
                                )}
                                <Button
                                  size="icon"
                                  variant="accent"
                                  title={t("复制")}
                                  onClick={async () => {
                                    try {
                                      await navigator.clipboard.writeText(item.text);
                                      showToast(t("已复制"));
                                    } catch {
                                      showToast(t("复制失败"), "error");
                                    }
                                  }}
                                >
                                  <Copy size={14} />
                                </Button>
                              </>
                            )}
                            <Button
                              size="icon"
                              variant="accent"
                              title={t("删除")}
                              onClick={async () => {
                                try {
                                  await deleteHistory(item.ts);
                                  setHistory((p) => p.filter((h) => h.ts !== item.ts));
                                  showToast(t("已删除"));
                                } catch {
                                  showToast(t("删除失败"), "error");
                                }
                              }}
                            >
                              <Trash2 size={14} />
                            </Button>
                          </div>
                        </div>
                      </div>
                    );
                  });
                })()}
                <button
                  type="button"
                  className="w-full flex items-center justify-center gap-1 py-3 text-sm text-text-muted hover:text-text-dim cursor-pointer bg-transparent border-0 font-inherit transition-colors"
                  onClick={() => {
                    const n = days + 3;
                    setDays(n);
                    loadHist(n);
                  }}
                >
                  {t("加载更多")}
                </button>
              </>
            )}
          </SectionItemList>
        </SectionContent>
      </Section>
      <Toast toast={toast} />
    </PageLayout>
  );
}

/* ---------- card ---------- */

function StatCard({ value, unit, label }: { value: string; unit: string; label: string }) {
  return (
    <div className="flex min-h-28 py-4 px-4 pb-5 bg-surface-card border border-border rounded-xl">
      <div className="flex flex-1 flex-col justify-between min-w-0">
        <div className="flex items-baseline gap-1 min-w-0">
          <span className="text-xl font-bold leading-[1.05] text-text tracking-normal whitespace-nowrap">
            {value}
          </span>
          {unit && <span className="text-sm font-medium text-text whitespace-nowrap">{unit}</span>}
        </div>
        <span className="text-xs font-semibold text-text whitespace-nowrap leading-none">
          {label}
        </span>
      </div>
    </div>
  );
}

function formatDurationParts(s: number): { value: string; unit: string } {
  const r = Math.round(s);
  if (r < 60) return { value: String(r), unit: "秒" };
  const m = Math.floor(r / 60);
  if (m < 60) return { value: String(m), unit: "分钟" };
  const h = r / 3600;
  return { value: h < 10 ? h.toFixed(1) : String(Math.round(h)), unit: "小时" };
}
