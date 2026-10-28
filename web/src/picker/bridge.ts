/** Typed IPC for the multi-version picker window. */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface PickerOptionView {
  id: string;
  title: string;
  /** Formatted accelerator label ("R ⌥", "T"), or empty when unbound. */
  hotkey: string;
  kind: "original" | "template";
  status: "ready" | "loading" | "failed";
  text?: string | null;
  error?: string | null;
}

export interface PickerView {
  sessionId: number;
  options: PickerOptionView[];
  delivering: boolean;
  allCompleted: boolean;
  error?: string | null;
  theme: string;
  targetApp?: string | null;
  /**
   * Cards per row, computed natively from the target display's work area.
   * Authoritative: `window.innerWidth` can lag a cross-monitor resize of the
   * reused picker window, which would otherwise briefly (or persistently)
   * render the previous monitor's column count.
   */
  columns?: number;
}

export function getPickerState(): Promise<PickerView | null> {
  return invoke("get_picker_state");
}

export function selectPickerOption(sessionId: number, optionId: string): Promise<void> {
  return invoke("select_picker_option", { sessionId, optionId });
}

export function cancelPicker(sessionId: number): Promise<void> {
  return invoke("cancel_picker", { sessionId });
}

/** Subscribe to session/result updates. Returns an unlisten cleanup. */
export function onPickerState(cb: (view: PickerView) => void): () => void {
  let active = true;
  const unlisten = listen<PickerView>("picker:state", (event) => {
    if (active) cb(event.payload);
  });
  return () => {
    active = false;
    unlisten.then((fn) => fn());
  };
}

/** Payload of "picker:closing" — the direction lets a failed delivery undo the
 * exit animation on the same session (`closing: false` = window is back). */
export interface PickerClosingSignal {
  generation?: number;
  closing?: boolean;
}

/**
 * Subscribe to close/reopen announcements for the window.
 *
 * The window itself is shown/hidden natively, so the renderer cannot animate an
 * exit on its own timeline: the backend announces the close at the moment the
 * user's selection or cancel is accepted and keeps the window on screen just
 * long enough for the pop-out to play. A `closing: false` signal means a
 * re-arm (failed delivery) put the session back on screen.
 */
export function onPickerClosing(cb: (signal: PickerClosingSignal) => void): () => void {
  let active = true;
  const unlisten = listen<PickerClosingSignal>("picker:closing", (event) => {
    if (active) cb(event.payload ?? {});
  });
  return () => {
    active = false;
    unlisten.then((fn) => fn());
  };
}
