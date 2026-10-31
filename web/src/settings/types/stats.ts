/** Stats payload returned by the `get_stats` command. */

/** One local day's delivered usage (mirrors `stats::DailyUsage`). */
export interface DailyUsage {
  sessions: number;
  words: number;
  recordingMs: number;
}

export interface Stats {
  firstUsedAt?: string;
  /** Dictation attempts — every recording that opened the microphone. */
  totalUses?: number;
  /** Successfully delivered sessions. */
  totalSessions?: number;
  totalCharacters?: number;
  dailyCounts?: Record<string, number>;
  dailyStats?: Record<string, DailyUsage>;
}
