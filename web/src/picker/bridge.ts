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
   * Cards per row the native layout *asked for*, computed from the target
   * display's work area with the same policy that sized the panel. The
   * renderer cross-checks it against its actual viewport — the only direct
   * measurement of what really fits — and shrinks it when the viewport is
   * narrower than requested (e.g. AppKit constrained the frame).
   */
  columns?: number;
}

/**
 * Report the viewport the renderer actually measures plus the grid it applied,
 * so the recording log can compare desired layout with real layout per session.
 * Best-effort diagnostics: failures are silently ignored.
 */
export function reportPickerLayout(payload: {
  sessionId: number;
  viewportWidth: number;
  viewportHeight: number;
  scaleFactor: number;
  desiredColumns: number;
  columns: number;
}): void {
  invoke("report_picker_layout", {
    sessionId: payload.sessionId,
    viewportWidth: payload.viewportWidth,
    viewportHeight: payload.viewportHeight,
    scaleFactor: payload.scaleFactor,
    desiredColumns: payload.desiredColumns,
    columns: payload.columns,
  }).catch(() => {});
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
