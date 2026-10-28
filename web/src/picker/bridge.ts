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

/**
 * Subscribe to "the window is about to be hidden".
 *
 * The window itself is shown/hidden natively, so the renderer cannot animate an
 * exit on its own: the backend announces the close and keeps the window on screen
 * just long enough for the pop-out transition to play.
 */
export function onPickerClosing(cb: () => void): () => void {
  let active = true;
  const unlisten = listen("picker:closing", () => {
    if (active) cb();
  });
  return () => {
    active = false;
    unlisten.then((fn) => fn());
  };
}
