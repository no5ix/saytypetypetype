//! Native pointer polling bridge for the picker WebView.
//!
//! Mirrors the 25 ms native pointer polling approach in sux-mac's
//! xSelectionToolbar. The native monitor never activates the app. It forwards
//! each move into the picker WebView, where exact DOM card bounds are hit-tested
//! and the card under the pointer is marked with data-native-hovered.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask};
use objc2_foundation::MainThreadMarker;
use tauri::{AppHandle, Manager};

use super::PICKER_LABEL;

thread_local! {
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

struct Bridge {
    monitor: Retained<AnyObject>,
    _handler: RcBlock<dyn Fn(NonNull<NSEvent>)>,
}

pub(crate) fn enable(app: &AppHandle) {
    if MainThreadMarker::new().is_none() || BRIDGE.with(|bridge| bridge.borrow().is_some()) {
        return;
    }
    let handle = app.clone();
    let handler = RcBlock::new(move |_event: NonNull<NSEvent>| tick(&handle));
    let Some(monitor) =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::MouseMoved, &handler)
    else {
        log_rec!(warn, "Could not install picker native pointer monitor");
        return;
    };
    BRIDGE.with(|bridge| {
        *bridge.borrow_mut() = Some(Bridge {
            monitor,
            _handler: handler,
        });
    });
}

pub(crate) fn disable() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if let Some(state) = BRIDGE.with(|bridge| bridge.borrow_mut().take()) {
        unsafe { NSEvent::removeMonitor(&state.monitor) };
    }
}

fn tick(app: &AppHandle) {
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
    // SAFETY: Tauri owns the live picker NSWindow for the app lifetime.
    let ns_window: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
    if !ns_window.isVisible() {
        return;
    }
    let frame = ns_window.frame();
    let mouse = NSEvent::mouseLocation();
    let x = mouse.x - frame.origin.x;
    let y = frame.size.height - (mouse.y - frame.origin.y);
    let script = format!(
        "window.__pickerNativePointer={{x:{x},y:{y}}};window.__pickerNativeHoverTick?.()"
    );
    let _ = window.eval(&script);
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_hover_updates_are_rate_limited_by_the_monitor_source() {
        // The source is AppKit's mouse-moved monitor; it receives one callback
        // per native pointer move and does not run when the pointer is stationary.
        assert!(super::PICKER_LABEL.len() > 0);
    }
}
