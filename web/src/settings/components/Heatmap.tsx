import { useI18n } from "@/i18n";
import { Section, SectionContent, SectionHeader } from "@/settings/layout/PageLayout";
import type { DailyUsage } from "@/settings/types/stats";

/* ---------- constants ---------- */

const WEEKS = 26;
const MONTHS_EN = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/* ---------- helpers ---------- */

function heatLevel(count: number, sorted: number[]): number {
  if (!count) return 0;
  if (!sorted.length) return 1;
  const p25 = sorted[Math.floor(sorted.length * 0.25)];
  const p50 = sorted[Math.floor(sorted.length * 0.5)];
  const p75 = sorted[Math.floor(sorted.length * 0.75)];
  if (count <= p25) return 1;
  if (count <= p50) return 2;
  if (count <= p75) return 3;
  return 4;
}

const HEAT_BG: Record<number, string> = {
  0: "bg-heatmap-0",
  1: "bg-heatmap-1",
  2: "bg-heatmap-2",
  3: "bg-heatmap-3",
  4: "bg-heatmap-4",
};

/** Local `YYYY-MM-DD` key for a date (never UTC — the heatmap is a local-day view). */
export function dayKey(dt: Date): string {
  return `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, "0")}-${String(
    dt.getDate(),
  ).padStart(2, "0")}`;
}

/**
 * Microphone time in the coarsest unit that keeps the number readable, matching
 * the dashboard cards so the tooltip and the cards never disagree.
 */
export function formatRecording(ms: number, unit: (u: string) => string): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}${unit("秒")}`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}${unit("分钟")}`;
  const hours = minutes / 60;
  return `${hours < 10 ? hours.toFixed(1) : Math.round(hours)}${unit("小时")}`;
}

/* ---------- component ---------- */

interface HeatmapProps {
  dailyCounts: Record<string, number>;
  /** Per-day sessions/words/mic time; falls back to `dailyCounts` when absent. */
  dailyStats?: Record<string, DailyUsage>;
}

export function Heatmap({ dailyCounts, dailyStats }: HeatmapProps) {
  const { language, t } = useI18n();
  const now = new Date();
  const start = new Date(now);
  start.setDate(start.getDate() - start.getDay());
  start.setDate(start.getDate() - (WEEKS - 1) * 7);

  const monthLabel = (month: number) => (language === "en" ? MONTHS_EN[month] : `${month + 1}月`);
  const dateLabel = (dt: Date) =>
    language === "en"
      ? `${MONTHS_EN[dt.getMonth()]} ${dt.getDate()}`
      : `${dt.getMonth() + 1}月${dt.getDate()}日`;

  // Intensity follows delivered words (the metric the cards headline); older
  // stats files only have character counts, which is the same shape of signal.
  const wordsFor = (key: string) => dailyStats?.[key]?.words ?? dailyCounts[key] ?? 0;
  const counts = Object.keys(dailyStats || dailyCounts)
    .map(wordsFor)
    .filter((c) => c > 0)
    .sort((a, b) => a - b);

  let totalWords = 0;

  const cells: { key: string; count: number; date: Date; future: boolean }[] = [];
  for (let w = 0; w < WEEKS; w++) {
    for (let d = 0; d < 7; d++) {
      const dt = new Date(start);
      dt.setDate(dt.getDate() + w * 7 + d);
      const key = dayKey(dt);
      const c = wordsFor(key);
      totalWords += c;
      cells.push({ key, count: c, date: dt, future: dt > now });
    }
  }

  // Month label positions
  const mPos: Record<number, number> = {};
  let cm = -1;
  for (let w = 0; w < WEEKS; w++) {
    const dt = new Date(start);
    dt.setDate(dt.getDate() + w * 7);
    const m = dt.getMonth();
    if (m !== cm) {
      mPos[m] = w;
      cm = m;
    }
  }

  const tooltip = (c: { key: string; count: number; date: Date }) => {
    const usage = dailyStats?.[c.key];
    const date = dateLabel(c.date);
    if (!usage) {
      return c.count > 0 ? `${date} · ${t("{count} 字", { count: c.count })}` : date;
    }
    const parts = [
      t("{count} 次会话", { count: usage.sessions }),
      t("{count} 字", { count: usage.words }),
      `${t("录音")} ${formatRecording(usage.recordingMs, t)}`,
    ];
    return `${date} · ${parts.join(" · ")}`;
  };

  return (
    <Section>
      <SectionHeader title={t("使用统计")} />
      <SectionContent className="px-4 flex flex-wrap gap-4 items-end">
        {/* Grid area */}
        <div className="flex-1 min-w-0 overflow-x-auto">
          {/* Month labels */}
          <div className="relative h-3.5 ml-5.5 mb-1">
            {Object.entries(mPos).map(([m, w]) => (
              <span
                key={m}
                className="absolute text-[10px] text-text-muted"
                style={{ left: `${w * 14}px` }}
              >
                {monthLabel(+m)}
              </span>
            ))}
          </div>

          {/* Grid + day labels */}
          <div className="flex gap-2 w-max">
            {/* Day-of-week labels */}
            <div
              className="grid gap-0.75 text-[10px] text-text-muted w-3.5 shrink-0"
              style={{ gridTemplateRows: "repeat(7, 11px)" }}
            >
              {[
                { key: "sun", label: "" },
                { key: "mon", label: language === "en" ? "M" : t("一") },
                { key: "tue", label: "" },
                { key: "wed", label: language === "en" ? "W" : t("三") },
                { key: "thu", label: "" },
                { key: "fri", label: language === "en" ? "F" : t("五") },
                { key: "sat", label: "" },
              ].map((day) => (
                <span key={day.key} className="h-2.75 flex items-center justify-end leading-none">
                  {day.label}
                </span>
              ))}
            </div>

            {/* The 7×26 grid */}
            <div
              className="grid gap-0.75"
              style={{
                gridTemplateRows: "repeat(7, 11px)",
                gridAutoFlow: "column",
                gridAutoColumns: "11px",
              }}
            >
              {cells.map((c) => (
                <div
                  key={c.key}
                  className={`w-2.75 h-2.75 rounded-xs ${c.future ? "invisible" : HEAT_BG[heatLevel(c.count, counts)] || "bg-heatmap-0"}`}
                  title={c.future ? "" : tooltip(c)}
                />
              ))}
            </div>
          </div>
        </div>

        {/* Stats */}
        <div className="flex flex-col items-end gap-0.5 pb-0.5 shrink-0 whitespace-nowrap">
          <span className="text-sm text-text-muted">{t("过去 {weeks} 周", { weeks: WEEKS })}</span>
          <span className="text-sm text-text-dim">
            {t("共输入")} <strong>{totalWords.toLocaleString()}</strong> {t("字")}
          </span>
        </div>
      </SectionContent>
    </Section>
  );
}
