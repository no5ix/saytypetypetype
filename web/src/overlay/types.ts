export type AppState = "idle" | "connecting" | "recording" | "finishing";
export type HintLevel = "info" | "error" | "warn";
export interface Shortcut {
  key: string;
  label: string;
}
export interface OverlayState {
  finalText: string;
  partialText: string;
  hintText: string;
  hintLevel: HintLevel;
  hintVariant: string;
  appState: AppState;
  shortcuts: Shortcut[];
  theme: "system" | "light" | "dark";
}
export const INITIAL_OVERLAY_STATE: OverlayState = {
  finalText: "",
  partialText: "",
  hintText: "",
  hintLevel: "info",
  hintVariant: "text",
  appState: "idle",
  shortcuts: [],
  theme: "system",
};
