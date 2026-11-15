//! Native hover tracking for the non-activating picker window.
//!
//! Uses the coordinate-based strategy from no5ix/sux-mac: observe global mouse
//! movement while the picker is visible, hit-test the pointer against the
//! picker's current card rectangles in the WebView, and apply a direct visual
//! highlight. This avoids depending on DOM :hover delivery to a non-key window.

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
        tracing::warn!("Could not install the picker global mouse monitor");
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
    let script = "window.__pickerNativeHoverTick?.()";
    let _ = window.eval(script);
}
