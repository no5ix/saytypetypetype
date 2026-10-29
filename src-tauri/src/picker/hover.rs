//! Hover bridge for the non-activating picker window.
//!
//! The picker window is deliberately `.focusable(false)` and the app normally
//! runs as an inactive accessory process so the target application keeps its
//! keyboard focus. macOS only delivers continuous mouse-moved events to the
//! active application, so while the app is inactive a freshly shown picker
//! receives no moves and the DOM `hover:` styles never engage — the reported
//! "hovering a card does nothing until I click the header once" symptom (the
//! click is what activates the app).
//!
//! Fix: while the picker is visible, a global `NSEvent` monitor observes the
//! mouse-moved events the *active* application receives (read-only; the other
//! app is untouched), and for every move inside the picker's frame synthesizes
//! one local move bound to the picker's window number and dispatches it
//! straight to that `NSWindow`. Hover therefore works whether the picker is
//! key or not, and the app is never activated, so target-app focus is
//! preserved. Stale hover is self-correcting: each session remounts the card
//! list (`key={view.sessionId}` in the renderer), and the monitor is removed
//! on hide.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSWindow};
use objc2_foundation::{MainThreadMarker, NSPoint};
use tauri::{AppHandle, Manager};

use super::PICKER_LABEL;

/// Points of slack for the frame-containment test, mirroring the picker's
/// geometry verifier: a cursor grazing the frame edge is treated as outside.
const EDGE_SLACK: f64 = 2.0;

thread_local! {
    /// Installed monitor + handler, kept alive exactly while the picker shows.
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

struct Bridge {
    monitor: Retained<AnyObject>,
    // The handler block must outlive the registered monitor.
    _handler: RcBlock<dyn Fn(NonNull<NSEvent>)>,
}

/// Whether a screen point (bottom-left AppKit space) lies inside a window
/// frame (same space), with [`EDGE_SLACK`] inset.
fn frame_contains(origin: (f64, f64), size: (f64, f64), point: (f64, f64)) -> bool {
    point.0 > origin.0 + EDGE_SLACK
        && point.0 < origin.0 + size.0 - EDGE_SLACK
        && point.1 > origin.1 + EDGE_SLACK
        && point.1 < origin.1 + size.1 - EDGE_SLACK
}

/// Start the monitor while the picker window is visible. Idempotent; must run
/// on the main thread (the global-monitor handler also runs there, and both
/// AppKit calls below require it).
pub(crate) fn enable(app: &AppHandle) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if BRIDGE.with(|bridge| bridge.borrow().is_some()) {
        return;
    }
    let handle = app.clone();
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        // SAFETY: AppKit hands a valid mouse event to a MouseMoved global
        // monitor; we only read its timestamp for the synthetic event.
        let timestamp = unsafe { event.as_ref() }.timestamp();
        tick(&handle, timestamp);
    });
    let monitor =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::MouseMoved, &handler);
    BRIDGE.with(|bridge| {
        *bridge.borrow_mut() = Some(Bridge {
            monitor: monitor.expect("global NSEvent monitor registration"),
            _handler: handler,
        });
    });
}

/// Tear the monitor down (the picker hid). Main thread; idempotent.
pub(crate) fn disable() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if let Some(state) = BRIDGE.with(|bridge| bridge.borrow_mut().take()) {
        unsafe { NSEvent::removeMonitor(&state.monitor) };
    }
}

/// One observed global move: if the cursor is over the visible picker window,
/// hand a synthetic `mouseMoved` (in the picker's own base coordinates) to
/// that window so WKWebView's tracking machinery updates DOM hover. Real moves
/// still reach the webview when the app is active (`acceptsMouseMovedEvents`
/// on the window, see `place_and_show`), so the bridge only supplements the
/// inactive case and duplicate delivery cannot occur.
fn tick(app: &AppHandle, timestamp: f64) {
    // The global handler runs on the main thread; guard anyway since every
    // AppKit call below requires it.
    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = app.get_webview_window(PICKER_LABEL) else {
        return;
    };
    let Ok(ptr) = window.ns_window() else {
        return;
    };
    if ptr.is_null() {
        return;
    }
    // SAFETY: the pointer is the picker window's live NSWindow owned by tauri
    // for the app lifetime; we only read the frame and window number.
    let ns_window: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
    let frame = ns_window.frame();
    let mouse = NSEvent::mouseLocation();
    if !frame_contains(
        (frame.origin.x, frame.origin.y),
        (frame.size.width, frame.size.height),
        (mouse.x, mouse.y),
    ) {
        return;
    }
    let location = NSPoint {
        x: mouse.x - frame.origin.x,
        y: mouse.y - frame.origin.y,
    };
    let synthetic =
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::MouseMoved,
            location,
            NSEvent::modifierFlags_class(),
            timestamp,
            ns_window.windowNumber(),
            None,
            0,
            0,
            0.0,
        )
        .expect("synthetic NSEvent creation");
    // Direct `sendEvent:` dispatches the move to the view under the cursor
    // regardless of key-window status — the whole point of this bridge.
    unsafe {
        let _: () = msg_send![ns_window, sendEvent: &*synthetic];
    }
}

#[cfg(test)]
mod tests {
    use super::frame_contains;

    #[test]
    fn frame_contains_uses_a_small_edge_slack() {
        let origin = (100.0, -512.0);
        let size = (960.0, 540.0);
        assert!(frame_contains(origin, size, (580.0, -240.0)));
        // External display arranged above the primary: negative y accepted.
        assert!(frame_contains(origin, size, (140.0, -500.0)));
        assert!(!frame_contains(origin, size, (60.0, -240.0)));
        assert!(!frame_contains(origin, size, (100.0, -240.0))); // edge slack
        assert!(!frame_contains(origin, size, (580.0, 40.0)));
        // Below the frame bottom (secondary displays run either way).
        assert!(!frame_contains(origin, size, (580.0, -520.0)));
    }
}
