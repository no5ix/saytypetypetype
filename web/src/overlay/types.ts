export type AppState = "idle" | "connecting" | "recording" | "finishing";
export type HintLevel = "info" | "error" | "warn";
export interface OverlayState {
  finalText: string;
  partialText: string;
  hintText: string;
  hintLevel: HintLevel;
  hintVariant: string;
  appState: AppState;
  theme: "system" | "light" | "dark";
}
export const INITIAL_OVERLAY_STATE: OverlayState = {
  finalText: "",
  partialText: "",
  hintText: "",
  hintLevel: "info",
  hintVariant: "text",
  appState: "idle",
  theme: "system",
};
