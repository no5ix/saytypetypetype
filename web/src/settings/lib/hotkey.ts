/**
 * Hotkey display utilities extracted from settings.js.
 *
 * The backend records platform-agnostic physical key names
 * ("ControlLeft", "AltLeft", "MetaLeft", ...). We render them per OS: macOS uses
 * Apple symbols (⌃ ⇧ ⌥ ⌘), Windows uses its native labels (Ctrl / Shift / Alt /
 * Win). Callers pass `isMac` so the same recorded string displays correctly on
 * each platform.
 */

/** Alias entry: `[macSymbol, winLabel]`. The semantic Command / CmdOrCtrl tokens
 * resolve to Ctrl on Windows (matching the accelerator convention), while the
 * physical Meta* keys are the Windows logo key. */
const HOTKEY_ALIASES: Record<string, readonly [string, string]> = {
  CmdOrCtrl: ["⌘", "Ctrl"],
  CommandOrControl: ["⌘", "Ctrl"],
  Command: ["⌘", "Ctrl"],
  Cmd: ["⌘", "Ctrl"],
  Meta: ["⌘", "Ctrl"],
  Control: ["⌃", "Ctrl"],
  Ctrl: ["⌃", "Ctrl"],
  Shift: ["⇧", "Shift"],
  Alt: ["⌥", "Alt"],
  Option: ["⌥", "Alt"],
  Space: ["␣", "␣"],
  ControlLeft: ["L ⌃", "L Ctrl"],
  ControlRight: ["R ⌃", "R Ctrl"],
  ShiftLeft: ["L ⇧", "L Shift"],
  ShiftRight: ["R ⇧", "R Shift"],
  AltLeft: ["L ⌥", "L Alt"],
  AltRight: ["R ⌥", "R Alt"],
  MetaLeft: ["L ⌘", "L Win"],
  MetaRight: ["R ⌘", "R Win"],
};

/** Map a raw key name to its display symbol for the given platform. Defaults to
 * macOS symbols to preserve prior behavior for callers that omit the platform. */
export function normalizeHotkeyLabel(key: string, isMac = true): string {
  const entry = HOTKEY_ALIASES[key];
  if (!entry) return key;
  return isMac ? entry[0] : entry[1];
}

/** A hotkey token split into a main key cap and an optional left/right side
 * badge, e.g. ControlLeft -> { main: "⌃", side: "L" }. */
export interface HotkeyToken {
  main: string;
  side?: string;
}

/** Physical left/right modifier keys: `[mainSymbol@mac, mainLabel@win]`. The
 * L/R side badge is derived from the Left/Right suffix of the key name. */
const SIDED_MODIFIERS: Record<string, readonly [string, string]> = {
  ControlLeft: ["⌃", "Ctrl"],
  ControlRight: ["⌃", "Ctrl"],
  ShiftLeft: ["⇧", "Shift"],
  ShiftRight: ["⇧", "Shift"],
  AltLeft: ["⌥", "Alt"],
  AltRight: ["⌥", "Alt"],
  MetaLeft: ["⌘", "Win"],
  MetaRight: ["⌘", "Win"],
};

/** Normalize one hotkey token into a main cap label plus an optional L/R side
 * badge for the KeyCap's top-right corner. Trims surrounding whitespace so both
 * "ControlLeft" and " ControlLeft " (from " + "-joined strings) resolve. */
export function normalizeHotkeyToken(key: string, isMac = true): HotkeyToken {
  const trimmed = key.trim();
  const sided = SIDED_MODIFIERS[trimmed];
  if (sided) {
    return {
      main: isMac ? sided[0] : sided[1],
      side: trimmed.endsWith("Left") ? "L" : "R",
    };
  }
  return { main: normalizeHotkeyLabel(trimmed, isMac) };
}

/**
 * Split a `+`-joined accelerator into its display tokens.
 *
 * A lone key keeps its left/right badge ("R ⌥" is a real choice between two
 * physical keys). Inside a combination the badge is noise — "L ⌃ + L ⌘ + L"
 * reads as three L keys — so a chord renders as "⌃ + ⌘ + L" / "Ctrl + Win + L",
 * matching the native `format_hotkey_label` used by the result panel.
 */
export function formatHotkeyTokens(value: string, isMac = true): HotkeyToken[] {
  const raw = value
    .split("+")
    .map((token) => token.trim())
    .filter((token) => token.length > 0);
  const chord = raw.length > 1;
  return raw.map((token) => {
    const parsed = normalizeHotkeyToken(token, isMac);
    return chord ? { main: parsed.main } : parsed;
  });
}

/**
 * Format a prompt hotkey (array of physical key name strings, e.g.
 * ["Control", "Shift", "A"]) into a " + "-joined display string. Non-string
 * entries are dropped (defensive against malformed stored data).
 */
export function formatPromptHotkey(hotkey: unknown): string {
  return typeof hotkey === "string" ? hotkey : "";
}
