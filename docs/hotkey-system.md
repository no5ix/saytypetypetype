# Global Hotkey System

## Overview

SayTypeTypeType uses the `keytap` crate for global hotkey registration, replacing `tauri-plugin-global-shortcut`. This enables modifier-only hotkeys, left/right modifier distinction, and lower latency via raw keyboard event streaming.

## Architecture

```
┌─────────────────────────────────────────────────┐
│             HotkeyManager                        │
│  (owns nothing — listener thread runs forever)   │
└────────────────────┬────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────┐
│         HotkeyConfig (Arc<RwLock<...>>)          │
│  ┌────────────────────────────────────────────┐  │
│  │  bindings: Vec<HotkeyBinding>               │  │
│  │  matcher: MatcherState   (chord state machine)│
│  │  escape_enabled: bool                       │  │
│  │  tap_active: bool                           │  │
│  │  recording / record_tx   (UI recorder relay) │  │
│  └────────────────────────────────────────────┘  │
└────────────────────┬────────────────────────────┘
                     │ shared with listener thread
                     ▼
┌─────────────────────────────────────────────────┐
│     Listener Thread ("voicepaste-hotkey")        │
│  ┌────────────────────────────────────────────┐  │
│  │  run_listener_loop(tap, config, app_handle) │  │
│  │                                            │  │
│  │  loop {                                    │  │
│  │    tap.recv_timeout(100ms)                 │  │
│  │    → forward to recorder if recording      │  │
│  │    → handle Escape cancellation            │  │
│  │    → matcher.process(event) → HotkeyAction │  │
│  │    → dispatch Start/StopRecording to async │  │
│  │  }                                         │  │
│  └────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

### HotkeyBinding

```rust
struct HotkeyBinding {
    keys: BTreeSet<Key>,     // All keys that must be held simultaneously
    mode: HotkeyMode,        // Toggle or Hold
    prompt_id: Option<String>, // None = raw input, Some(id) = prompt hotkey
    wake_only: bool,         // Global toggle start; never a stop binding
}
```

## Hotkey Modes

```mermaid
stateDiagram-v2
    [*] --> Idle

    state "Toggle Mode" as Toggle {
        Idle --> Recording : keyup cycle completes (start)
        Recording --> Idle : keyup cycle completes (stop)
        Recording --> Idle : escape
    }

    state "Hold Mode" as Hold {
        Idle --> Recording : exact complete chord held for 500ms
        Recording --> Idle : release (any chord key)
        Recording --> Idle : escape
    }
```

### Matching and dispatch

`MatcherState` consumes the ordered keytap stream and a monotonic clock. Toggle
wakes on release of an exact global-key cycle. Every key in that cycle must
belong to the same fully pressed combination; releasing an extra key cannot
restore a shorter candidate. All ordinary key events are observed, including
letters outside registered shortcuts. Unbound lock keys invalidate a candidate
without waiting for potentially missing lock-key release events.

The global `app.hotkey` always starts toggle recordings. `app.voice_hotkey`
selects the raw-input stop key in toggle mode, or the independent hold shortcut
in hold mode. Prompt toggle keys are stop-only. Only the raw toggle stop may
share the global key. Other duplicate bindings are invalid; containment is
allowed only between toggle bindings. Hold shortcuts require a non-typing key
(function row, Tab or modifier), matching the recorder's first-key restriction.

For hold mode, the 500ms deadline begins when the **entire configured chord**
is pressed. Holding Ctrl for 200ms or 600ms before pressing C does not count
against the Ctrl+C deadline. An extra key or an early release cancels the
candidate until all keys are released. Timers run in the listener's ordered loop,
not detached tasks. An event at the deadline is processed after the due start,
so an immediate release produces start then stop in that order.

On activation the hold binding and template are frozen. New keys cannot switch
or finalize that session. Releasing any original member ends it; ESC cancels.
Toggle sessions accept only their captured toggle stop bindings. The original
bindings remain in effect for the active session across settings changes.

Actions carry explicit modes. A single dispatcher orders microphone startup
before a queued release; ASR/LLM finalization runs separately, and new starts
while it is in progress are rejected. Cancel generations revoke queued starts.
The global key retains the existing retry behavior on retryable failures. If no
toggle stop binding is enabled, it only retries failures and cannot start a new
recording that has no normal stop key.

After two seconds without keyboard events, native key-state queries may reclaim
**confirmed released** keys. Silence alone never clears a key. Unknown states
are retained. A lost release in an active hold cancels rather than submitting
possibly unintended output. macOS uses Core Graphics HID state and side-specific modifier flags (generic-only
modifier flags are unknown, not proof that one side was released); Windows uses
GetAsyncKeyState. These probes are recovery checks, not the hotkey event source.

### Escape Cancellation

When the state machine is in Connecting, Recording, or Finishing state, pressing Escape triggers `cancel_recording()`. The `escape_enabled` flag on `HotkeyConfig` is toggled by `sync_escape_shortcut()` during state transitions.

Escape uses a `was_pressed` guard to prevent key-repeat from triggering multiple cancellations.

Because ESC cancels out-of-band (not via a stop chord), `cancel_recording()` also calls `reset_recording()` to clear the matcher's session tracking, so the next keypress isn't mistaken for a stop.

## Hotkey Parsing

The `parse_hotkey_string()` function converts human-readable hotkey strings into `BTreeSet<Key>`:

| Format | Example | Result |
|--------|---------|--------|
| Function key | `"F13"` | `{Key::F13}` |
| Modifier + key | `"Control+Shift+A"` | `{ControlLeft, ShiftLeft, A}` |
| Side-specific | `"ShiftRight+F"` | `{ShiftRight, F}` |
| Aliases | `"Ctrl+C"`, `"Cmd+V"` | `{ControlLeft, C}`, `{MetaLeft, V}` |
| Cross-platform | `"CmdOrCtrl+S"` | `{MetaLeft, S}` (macOS) / `{ControlLeft, S}` (Windows) |
| Special keys | `"Space"`, `"Enter"`, `"Escape"` | Respective Key variants |

Supported modifier aliases: `Ctrl`/`Control`, `Shift`, `Alt`/`Option`, `Cmd`/`Super`/`Meta`/`Command`.
For side-specific modifiers: `ControlLeft`, `ShiftRight`, `AltLeft`, `CmdRight`, etc.

## Per-Prompt Hotkeys

Each prompt in `prompts.json` can have its own hotkey and mode:

```json
{
  "id": "polish-en",
  "name": "English Polish",
  "prompt": "Polish this text to sound professional.",
  "hotkey": "Control+Shift+E",
  "hotkey_mode": "hold"
}
```

Toggle recordings start neutral; the configured stop key chooses the final
prompt. Hold recordings freeze their prompt at activation and retain it until
release. `on_recording_stop` passes that prompt to the existing polishing flow.

Before polishing, `finalize_and_paste` validates the LLM config — if it's
incomplete (missing key/URL/model), it downgrades to raw text with a warning
instead of letting the call fail.

## Frontend Hotkey Recording

The settings UI calls the backend recorder through the resident keytap event
relay on both platforms, preserving physical left/right keys and press order.
ESC or the 10-second capture timeout leaves the previous setting unchanged.
Global and hold fields enforce the first-key restriction. The shared backend
binding compiler validates conflicts before saving; rejected edits show a toast.
Startup uses the same compiler and disables conflicting bindings while preserving
saved values. The global key and its allowed raw stop alias take priority.
Settings reports disabled bindings once per visit through a toast. Empty bindings
are normal disabled states. Switching mode keeps valid keys; if the new mode
invalidates the key, settings clears that field, saves the new mode and reports
the reason through a toast. Ordinary key edits still reject invalid replacements.

Legacy raw toggle keys become explicit `voice_hotkey` values. Legacy raw hold
mode keeps the global key but leaves `voice_hotkey` empty for reconfiguration.
The migration only runs when the new field is absent.

### Display: native modifier glyphs

Recorded combinations render through the `KeyCap` component (`settings/components/KeyCap.tsx`), which shows platform-native modifier symbols instead of generic text:

- **macOS**: ⌘ (Cmd), ⌃ (Control), ⇧ (Shift), ⌥ (Option/Alt)
- **Windows**: Ctrl, Alt, Shift, Win (Super)

The symbol set is chosen per-platform so the on-screen keycaps match the user's physical keyboard labels.

## Hot Reload

When the user saves settings:
1. `save_config_object()` writes `config.yaml`
2. `reload_hotkey_bindings()` is called, which rebuilds the binding list from the new config
3. The listener thread picks up the new bindings on its next iteration (no restart needed)
4. Pending timers are cancelled; active sessions retain their original mode, template and stop bindings
5. If bindings haven't changed, the reload is a no-op

### Permission Recovery

If `keytap::Tap::new()` fails with `PermissionDenied`, the app starts without hotkeys rather than crashing. The user can grant Accessibility/Input Monitoring permission and use the "Reinitialize" button in settings, which calls `ensure_hotkey_active()` to create a new tap.
