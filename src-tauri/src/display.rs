//! Display selection for the dictation indicator and the version picker.
//!
//! The app must place transient UI on the display that contains the window of
//! the application that will receive the dictated text — not always on the
//! primary display. This module keeps the geometry and selection rules pure and
//! unit-testable; the macOS implementation resolves the frontmost application's
//! front window and `NSScreen` work areas once per session.
//!
//! Coordinates are top-left global points, matching the CoreGraphics window
//! bounds used by the macOS resolver and Tauri's logical window positions. This
//! keeps secondary displays at negative x/y working without per-display math.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub(crate) fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub(crate) fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// Overlap area with another rect. Used to decide which display owns a
    /// window that straddles two screens.
    pub(crate) fn intersection_area(&self, other: &Rect) -> f64 {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        (right - left).max(0.0) * (bottom - top).max(0.0)
    }
}

/// One screen and its usable area (menu bar and Dock excluded).
///
/// `frame`/`work_area` are logical points (top-left global space). The
/// `scale_factor` is the screen's backing-store ratio (points → pixels); it is
/// carried for diagnostics and containment math only — the sizing formulas work
/// purely in points and never multiply by it. Mixing the two is exactly what
/// made the picker come out wrong when sessions moved between a Retina display
/// (2×) and an external monitor (1×).
///
/// `work_area_bl` is the *same* visible rect in AppKit's bottom-left global
/// space, captured straight from `NSScreen.visibleFrame`: the native picker
/// placement feeds it to `setFrame:` with zero conversion, so the coordinates
/// AppKit consumes are never the product of a round-trip through our
/// top-left space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Display {
    pub name: String,
    pub frame: Rect,
    pub work_area: Rect,
    /// Visible frame in AppKit bottom-left global points (native placement space).
    pub work_area_bl: Rect,
    pub scale_factor: f64,
}

/// The frontmost application whose window we are targeting.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TargetApp {
    pub pid: i32,
    pub name: String,
    pub window: Option<Rect>,
}

/// A resolved session target: the chosen display plus the app it belongs to.
#[derive(Clone, Debug)]
pub(crate) struct TargetDisplay {
    pub display: Display,
    pub target: Option<TargetApp>,
}

/// Pick the display that owns most of `window`. Ties are broken by the first
/// display, which mirrors macOS's left-to-right, top-to-bottom screen order.
/// Returns `None` when there is no overlap at all, letting the caller fall back
/// deterministically (primary display).
pub(crate) fn display_for_window(window: Rect, displays: &[Display]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, display) in displays.iter().enumerate() {
        let area = display.frame.intersection_area(&window);
        if area <= 0.0 {
            continue;
        }
        if best.is_none_or(|(_, best_area)| area > best_area) {
            best = Some((index, area));
        }
    }
    best.map(|(index, _)| index)
}

/// Pick the display containing a point. Returns `None` when the point is
/// outside every screen (callers then use the deterministic primary fallback).
pub(crate) fn display_for_point(x: f64, y: f64, displays: &[Display]) -> Option<usize> {
    displays
        .iter()
        .position(|display| display.frame.contains(x, y))
}

/// Bottom-center position (top-left origin) inside a work area.
pub(crate) fn bottom_center_position(
    work_area: Rect,
    width: f64,
    height: f64,
    margin: f64,
) -> (f64, f64) {
    let x = work_area.x + (work_area.width - width) / 2.0;
    let y = work_area.y + work_area.height - height - margin;
    (x, y)
}

/// Bottom-center position in AppKit bottom-left coordinates: `(x, y)` is the
/// window's bottom-left corner, directly consumable by `NSWindow.setFrame:`
/// against the screen's raw `visibleFrame` — no origin conversion involved.
/// The window height is deliberately not a parameter: the bottom edge anchors
/// to `y + height` implicitly, so the margin applies from the screen bottom
/// without any arithmetic that a wrong primary height could corrupt.
pub(crate) fn bottom_center_position_bl(work_area_bl: Rect, width: f64, margin: f64) -> (f64, f64) {
    let x = work_area_bl.x + (work_area_bl.width - width) / 2.0;
    let y = work_area_bl.y + margin;
    (x, y)
}

/// Mirror of [`Rect`] between the two global spaces across a primary of height
/// `primary_height` (top-left y ↦ bottom-left origin y). Production code never
/// converts — it consumes each space's native numbers directly — so this
/// exists for the tests that prove both views describe the same rectangles.
#[cfg(test)]
pub(crate) fn mirror_vertical(rect: Rect, primary_height: f64) -> Rect {
    Rect {
        x: rect.x,
        y: primary_height - (rect.y + rect.height),
        width: rect.width,
        height: rect.height,
    }
}

/// The complete picker layout decision: native window size plus the exact card
/// grid the renderer must use. Columns are computed here once and shipped to
/// the frontend in the open payload, so the native window, the row count used
/// for sizing, and the rendered grid can never disagree — not even while a
/// reused window's `innerWidth` still reflects the previous monitor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickerLayout {
    pub width: f64,
    pub height: f64,
    /// Cards per row the renderer must use (already capped by card count).
    pub columns: usize,
    /// Rows the cards occupy at that column count (drives the height growth).
    pub rows: usize,
}

/// Deterministic column policy for the final window width.
///
/// * `< 520pt`  — one column (genuinely narrow fallback; keeps cards readable)
/// * `< 780pt`  — two columns
/// * `≥ 780pt`  — up to four columns
///
/// The tier boundaries match the renderer fallback in `PickerApp.tsx`, but the
/// authoritative value is always this function: every supported monitor (MacBook
/// built-in at 1280–1512pt, 1080p+ externals, ultrawides) lands three cards in
/// one horizontal row because their clamped three-card width (1080pt) is above
/// the two-column tier. Scaling factors never enter here — widths are points on
/// every display.
pub(crate) fn picker_columns(width: f64, option_count: usize) -> usize {
    let max_columns = if width < 520.0 {
        1
    } else if width < 780.0 {
        2
    } else {
        4
    };
    max_columns.min(option_count.max(1))
}

/// Full picker layout for a target work area and card count.
///
/// Every piece of picker geometry comes out of this one function so the native
/// window, the shipped column policy and the tests cannot disagree:
///
/// * width  — 120% of the historical per-count width, clamped into the work area
///   and capped at [`MAX_WIDTH`] so ultrawide displays keep readable card rows;
/// * height — three quarters of the target display's usable height, grown only
///   when the card rows genuinely need more, and clamped into the work area;
/// * columns/rows — the explicit [`picker_columns`] policy applied to the final
///   clamped width, shipped verbatim to the renderer.
///
/// Both axes keep a floor so the panel stays readable on small laptops and can
/// never exceed the visible area (`clamp_to` prefers the area limit over the
/// floor). The panel content itself stretches (`grid-auto-rows: minmax(…, 1fr)`),
/// so the extra space goes to the cards instead of empty gaps.
///
/// Everything is expressed in logical points end to end: `NSScreen` frames and
/// `CGWindowBounds` are already points, and the caller applies the result as a
/// Tauri `LogicalSize`/`LogicalPosition`. Retina backing scale never enters the
/// formula, so a session can never size against another monitor's pixels.
pub(crate) fn picker_layout(work_area: Rect, option_count: usize) -> PickerLayout {
    let base_width: f64 = match option_count {
        0 | 1 => 560.0,
        2 => 620.0,
        3 => 900.0,
        _ => 1120.0,
    };
    let width = clamp_to(
        base_width * WIDTH_GROWTH,
        MIN_WIDTH,
        (work_area.width - AREA_INSET).min(MAX_WIDTH),
    );

    // Deriving columns from the *clamped* width matters on narrow displays:
    // there the panel is narrower than its target, fits fewer cards per row and
    // therefore needs more rows of height.
    let columns = picker_columns(width, option_count);
    let rows = option_count.div_ceil(columns).max(1);

    let content_height = CHROME_HEIGHT + rows as f64 * ROW_HEIGHT;
    let height = clamp_to(
        (work_area.height * HEIGHT_RATIO).max(content_height),
        MIN_HEIGHT,
        work_area.height - AREA_INSET,
    );
    PickerLayout {
        width,
        height,
        columns,
        rows,
    }
}

/// Clamp `value` into `[min, max]`, preferring `max` when the area cannot even fit
/// `min`: staying inside the visible screen outranks the readability floor.
fn clamp_to(value: f64, min: f64, max: f64) -> f64 {
    if max < min {
        max.max(0.0)
    } else {
        value.clamp(min, max)
    }
}

/// Width multiplier over the historical per-count widths.
const WIDTH_GROWTH: f64 = 1.2;
/// Hard ceiling on the panel width: the widest historical per-count target
/// (1120pt for four-plus cards) times [`WIDTH_GROWTH`]. Without it an ultrawide
/// display would stretch the card grid far past the point where rows stay usable.
const MAX_WIDTH: f64 = 1344.0;
/// Target share of the display's usable height (three quarters).
const HEIGHT_RATIO: f64 = 0.75;
/// Readability floors.
const MIN_WIDTH: f64 = 480.0;
const MIN_HEIGHT: f64 = 360.0;
/// Keep the panel at least this far from the work-area edges.
const AREA_INSET: f64 = 48.0;
/// One card row: CSS `grid-auto-rows` minimum (148px) plus the grid gap (8px).
/// Reduced ~5% from 164pt alongside the renderer's `minmax` floor so the
/// rendered cards and the window sizing stay consistent.
const ROW_HEIGHT: f64 = 156.0;
/// Vertical space used by the window padding, the panel border, the header, the
/// footer and the scroll-region gaps (measured ~115.8pt in the renderer), i.e.
/// everything inside the window that is not a card row. The single grid gap is
/// already accounted for by [`ROW_HEIGHT`].
const CHROME_HEIGHT: f64 = 116.0;

/// Fallback work area when no screen information is available. Coordinates are
/// synthetic but deterministic; only used if AppKit unexpectedly returns no
/// screens.
pub(crate) fn fallback_display() -> Display {
    let frame = Rect {
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 900.0,
    };
    let work_area = Rect {
        y: 25.0,
        height: 875.0,
        ..frame
    };
    Display {
        name: "fallback".to_string(),
        work_area_bl: Rect {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 875.0,
        },
        frame,
        work_area,
        scale_factor: 1.0,
    }
}

/// Capture the frontmost application's display. `None` only when the caller is
/// off macOS (the native resolver is a no-op there).
#[cfg(target_os = "macos")]
pub(crate) fn capture_target_display() -> Option<TargetDisplay> {
    macos::capture()
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn capture_target_display() -> Option<TargetDisplay> {
    None
}

/// Resolve the display to use for a session, with a logged primary fallback.
pub(crate) fn resolve_target_display() -> TargetDisplay {
    if let Some(target) = capture_target_display() {
        if let Some(app) = target.target.as_ref() {
            log_app!(
                debug,
                "Dictation target: app={:?} pid={} window={:?} display={:?}",
                app.name,
                app.pid,
                app.window,
                target.display.frame
            );
        } else {
            log_app!(
                debug,
                "Dictation target window unavailable; using display {:?}",
                target.display.frame
            );
        }
        return target;
    }

    log_app!(
        warn,
        "No display information available; using deterministic primary fallback"
    );
    TargetDisplay {
        display: fallback_display(),
        target: None,
    }
}

/// Resolve the display on the main thread, reporting capture failure as `None`
/// and bounding the main-thread round trip. The picker uses this to re-detect
/// the target display at open time: monitors can be plugged, unplugged or
/// re-resolved between recording start and picker open, and the panel must be
/// sized for the screen it is about to appear on, not the one from session
/// start.
pub(crate) async fn try_resolve_target_display_on_main(
    app: &tauri::AppHandle,
) -> Option<TargetDisplay> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(capture_target_display());
        })
        .is_err()
    {
        return capture_target_display();
    }
    match tokio::time::timeout(Duration::from_secs(5), rx).await {
        Ok(Ok(target)) => target,
        Ok(Err(_)) | Err(_) => {
            log_app!(warn, "Display capture was cancelled or timed out");
            None
        }
    }
}

/// Resolve the display on the main thread. AppKit screen/workspace lookups are
/// safest there; the session start path runs on the async runtime.
pub(crate) async fn resolve_target_display_on_main(app: &tauri::AppHandle) -> TargetDisplay {
    try_resolve_target_display_on_main(app)
        .await
        .unwrap_or_else(|| {
            log_app!(warn, "Display capture unavailable; using primary fallback");
            TargetDisplay {
                display: fallback_display(),
                target: None,
            }
        })
}

/// Bring the session's target application back to the front before pasting.
/// Clicking the picker activates VoicePaste on macOS; without this the paste
/// keystroke would land in the picker instead of the user's window. No-op and
/// `false` when the target is unknown or this is not macOS.
#[cfg(target_os = "macos")]
pub(crate) fn activate_target_app(pid: i32) -> bool {
    macos::activate(pid)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn activate_target_app(_pid: i32) -> bool {
    false
}

/// Pid of the application frontmost right now, when it can be determined.
/// The picker resolves its paste destination with this at selection time, so
/// text follows the app the user is actually working in even if focus moved
/// while the panel was open.
#[cfg(target_os = "macos")]
pub(crate) fn frontmost_app_pid() -> Option<i32> {
    macos::frontmost_pid()
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn frontmost_app_pid() -> Option<i32> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CStr;

    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{NSRect, NSString};

    use super::{Display, Rect, TargetApp, TargetDisplay};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_window: u32) -> *mut AnyObject;
        static kCGWindowOwnerPID: *const AnyObject;
        static kCGWindowLayer: *const AnyObject;
        static kCGWindowAlpha: *const AnyObject;
        static kCGWindowBounds: *const AnyObject;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: *const std::ffi::c_void);
    }

    /// `kCGWindowListOptionOnScreenOnly`.
    const ON_SCREEN_ONLY: u32 = 1;
    /// `kCGWindowListExcludeDesktopElements`.
    const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;

    pub(super) fn capture() -> Option<TargetDisplay> {
        let displays = screens()?;
        let target = frontmost_window();
        let matched = target
            .as_ref()
            .and_then(|app| app.window)
            .and_then(|window| super::display_for_window(window, &displays))
            .or_else(|| {
                target
                    .as_ref()
                    .and_then(|app| app.window)
                    .and_then(|window| {
                        let (x, y) = window.center();
                        super::display_for_point(x, y, &displays)
                    })
            });
        // No overlap and no containing centre (minimised/off-screen target, or
        // permissions missing): fall back to the primary screen deliberately.
        let index = matched.unwrap_or(0);
        if matched.is_none() {
            log_app!(
                debug,
                "No display contains the target window; defaulting to primary {:?}",
                displays.get(index).map(|display| &display.name)
            );
        }

        Some(TargetDisplay {
            display: displays
                .get(index)
                .cloned()
                .unwrap_or_else(super::fallback_display),
            target,
        })
    }

    fn screens() -> Option<Vec<Display>> {
        unsafe {
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            if screens.is_null() {
                return None;
            }
            let count: usize = msg_send![screens, count];
            if count == 0 {
                return None;
            }

            // AppKit's global space has its primary screen (the one carrying the
            // menu bar) at origin (0, 0). Find it by identity rather than relying
            // on `screens[0]` ordering, and use its point height to mirror rects
            // into the top-left space used by CoreGraphics bounds.
            let mut raw = Vec::with_capacity(count);
            for index in 0..count {
                let screen: *mut AnyObject = msg_send![screens, objectAtIndex: index];
                let frame: NSRect = msg_send![screen, frame];
                let visible: NSRect = msg_send![screen, visibleFrame];
                // Points → backing pixels for this screen only. Diagnostic use;
                // the picker sizing formula stays in points.
                let scale: f64 = msg_send![screen, backingScaleFactor];
                raw.push((frame, visible, scale, screen_name(screen)));
            }
            let primary_height = raw
                .iter()
                .find(|(frame, ..)| frame.origin.x == 0.0 && frame.origin.y == 0.0)
                .or(raw.first())
                .map(|(frame, ..)| frame.size.height)
                .unwrap_or(900.0);

            let result = raw
                .into_iter()
                .map(|(frame, visible, scale, name)| Display {
                    name,
                    frame: to_top_left(frame, primary_height),
                    work_area: to_top_left(visible, primary_height),
                    // Unconverted AppKit space: exactly what setFrame: consumes.
                    work_area_bl: Rect {
                        x: visible.origin.x,
                        y: visible.origin.y,
                        width: visible.size.width,
                        height: visible.size.height,
                    },
                    scale_factor: if scale.is_finite() && scale > 0.0 {
                        scale
                    } else {
                        1.0
                    },
                })
                .collect();
            Some(result)
        }
    }

    unsafe fn screen_name(screen: *mut AnyObject) -> String {
        let name: *mut AnyObject = msg_send![screen, localizedName];
        if name.is_null() {
            return String::new();
        }
        let utf8: *const std::ffi::c_char = msg_send![name, UTF8String];
        if utf8.is_null() {
            return String::new();
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    }

    fn to_top_left(rect: NSRect, primary_height: f64) -> Rect {
        Rect {
            x: rect.origin.x,
            y: primary_height - (rect.origin.y + rect.size.height),
            width: rect.size.width,
            height: rect.size.height,
        }
    }

    fn frontmost_window() -> Option<TargetApp> {
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return None;
            }
            let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
            if app.is_null() {
                return None;
            }
            let pid: i32 = msg_send![app, processIdentifier];
            let name = localized_name(app);
            let window = window_bounds(pid);
            Some(TargetApp { pid, name, window })
        }
    }

    pub(super) fn frontmost_pid() -> Option<i32> {
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return None;
            }
            let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
            if app.is_null() {
                return None;
            }
            Some(msg_send![app, processIdentifier])
        }
    }

    unsafe fn localized_name(app: *mut AnyObject) -> String {
        let name: *mut AnyObject = msg_send![app, localizedName];
        if name.is_null() {
            return String::new();
        }
        let utf8: *const std::ffi::c_char = msg_send![name, UTF8String];
        if utf8.is_null() {
            return String::new();
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    }

    /// Bounds of the frontmost on-screen, layer-0 window owned by `pid`.
    /// CoreGraphics exposes bounds for every window without Screen Recording
    /// permission; window titles would require it, and are not needed.
    fn window_bounds(pid: i32) -> Option<Rect> {
        unsafe {
            let array = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, 0);
            if array.is_null() {
                return None;
            }
            let count: usize = msg_send![array, count];
            let mut first: Option<Rect> = None;

            for index in 0..count {
                let info: *mut AnyObject = msg_send![array, objectAtIndex: index];
                if info.is_null() {
                    continue;
                }
                let owner: *mut AnyObject = msg_send![info, objectForKey: kCGWindowOwnerPID];
                if owner.is_null() {
                    continue;
                }
                let owner_pid: i32 = msg_send![owner, intValue];
                if owner_pid != pid {
                    continue;
                }
                let layer: *mut AnyObject = msg_send![info, objectForKey: kCGWindowLayer];
                if !layer.is_null() {
                    let layer_value: i32 = msg_send![layer, intValue];
                    if layer_value != 0 {
                        continue;
                    }
                }
                let alpha: *mut AnyObject = msg_send![info, objectForKey: kCGWindowAlpha];
                if !alpha.is_null() {
                    let alpha_value: f64 = msg_send![alpha, doubleValue];
                    if alpha_value < 0.05 {
                        continue;
                    }
                }
                let Some(bounds) = bounds_for(info) else {
                    continue;
                };
                if bounds.width < 40.0 || bounds.height < 40.0 {
                    continue;
                }
                first = Some(bounds);
                break;
            }

            CFRelease(array as *const std::ffi::c_void);
            // The first on-screen window in CoreGraphics order is the frontmost.
            // It is the window the user is actually looking at, even when a
            // larger background window of the same app exists.
            first
        }
    }

    unsafe fn bounds_for(info: *mut AnyObject) -> Option<Rect> {
        let bounds: *mut AnyObject = msg_send![info, objectForKey: kCGWindowBounds];
        if bounds.is_null() {
            return None;
        }
        let x = number(bounds, "X")?;
        let y = number(bounds, "Y")?;
        let width = number(bounds, "Width")?;
        let height = number(bounds, "Height")?;
        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return None;
        }
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }

    unsafe fn number(dict: *mut AnyObject, key: &str) -> Option<f64> {
        let key = NSString::from_str(key);
        let value: *mut AnyObject = msg_send![dict, objectForKey: &*key];
        if value.is_null() {
            return None;
        }
        Some(msg_send![value, doubleValue])
    }

    /// `NSApplicationActivateIgnoringOtherApps` — the option that makes this
    /// reliable immediately after a click on our non-keyable picker window.
    const ACTIVATE_IGNORING_OTHER_APPS: usize = 1 << 1;

    pub(super) fn activate(pid: i32) -> bool {
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
            if app.is_null() {
                return false;
            }
            let activated: bool = msg_send![app, activateWithOptions: ACTIVATE_IGNORING_OTHER_APPS];
            activated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: f64, y: f64, width: f64, height: f64) -> Display {
        let frame = Rect {
            x,
            y,
            width,
            height,
        };
        // Real work areas exclude the menu bar; tests keep the same bounds
        // unless a test is specifically about positioning.
        let work_area = frame;
        Display {
            name: format!("screen@{x},{y}"),
            work_area_bl: mirror_vertical(work_area, TEST_PRIMARY_HEIGHT),
            frame,
            work_area,
            scale_factor: 1.0,
        }
    }

    /// Height of the synthetic primary used by the test fixtures, so the
    /// bottom-left mirrors built by [`display`] describe the same physical
    /// rectangles as the top-left values the positioning helpers consume.
    const TEST_PRIMARY_HEIGHT: f64 = 1080.0;

    /// A display with an explicit backing scale (Retina built-in vs 1× external).
    fn scaled_display(x: f64, y: f64, width: f64, height: f64, scale: f64) -> Display {
        Display {
            scale_factor: scale,
            ..display(x, y, width, height)
        }
    }

    /// Test tuple view of [`picker_layout`] (size only) for the older assertions.
    fn picker_window_size(work_area: Rect, option_count: usize) -> (f64, f64) {
        let layout = picker_layout(work_area, option_count);
        (layout.width, layout.height)
    }

    fn three_displays() -> Vec<Display> {
        vec![
            display(0.0, 0.0, 1920.0, 1080.0),
            display(1920.0, 0.0, 2560.0, 1440.0),
            display(-2560.0, 0.0, 2560.0, 1440.0),
        ]
    }

    #[test]
    fn chooses_primary_display_for_primary_window() {
        let displays = three_displays();
        let window = Rect {
            x: 200.0,
            y: 120.0,
            width: 900.0,
            height: 700.0,
        };
        assert_eq!(display_for_window(window, &displays), Some(0));
    }

    #[test]
    fn chooses_right_secondary_display() {
        let displays = three_displays();
        let window = Rect {
            x: 2300.0,
            y: 100.0,
            width: 1000.0,
            height: 700.0,
        };
        assert_eq!(display_for_window(window, &displays), Some(1));
    }

    #[test]
    fn chooses_left_secondary_display_with_negative_coordinates() {
        let displays = three_displays();
        let window = Rect {
            x: -2200.0,
            y: 200.0,
            width: 900.0,
            height: 700.0,
        };
        assert_eq!(display_for_window(window, &displays), Some(2));
    }

    #[test]
    fn chooses_display_above_primary_with_negative_y() {
        let mut displays = three_displays();
        displays.push(display(0.0, -1440.0, 1920.0, 1440.0));
        let window = Rect {
            x: 300.0,
            y: -1100.0,
            width: 1000.0,
            height: 800.0,
        };
        assert_eq!(display_for_window(window, &displays), Some(3));
    }

    #[test]
    fn chooses_display_with_largest_overlap_when_window_straddles() {
        let displays = three_displays();
        // 800 px on the primary, 100 px on the right display.
        let window = Rect {
            x: 1120.0,
            y: 100.0,
            width: 900.0,
            height: 500.0,
        };
        assert_eq!(display_for_window(window, &displays), Some(0));
    }

    #[test]
    fn unknown_window_falls_back_to_none() {
        let displays = three_displays();
        let window = Rect {
            x: 9000.0,
            y: 9000.0,
            width: 400.0,
            height: 400.0,
        };
        assert_eq!(display_for_window(window, &displays), None);
    }

    #[test]
    fn consecutive_sessions_can_target_different_displays() {
        let displays = three_displays();
        let primary = Rect {
            x: 100.0,
            y: 100.0,
            width: 800.0,
            height: 600.0,
        };
        let right = Rect {
            x: 2400.0,
            y: 100.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(display_for_window(primary, &displays), Some(0));
        assert_eq!(display_for_window(right, &displays), Some(1));
        assert_eq!(display_for_window(primary, &displays), Some(0));
    }

    #[test]
    fn bottom_center_uses_work_area_and_margin() {
        let work = Rect {
            x: 0.0,
            y: 25.0,
            width: 1920.0,
            height: 1055.0,
        };
        assert_eq!(
            bottom_center_position(work, 1120.0, 380.0, 24.0),
            (400.0, 676.0)
        );
    }

    #[test]
    fn bottom_center_supports_negative_display_origins() {
        let work = Rect {
            x: -2560.0,
            y: -1440.0,
            width: 2560.0,
            height: 1440.0,
        };
        assert_eq!(
            bottom_center_position(work, 1120.0, 380.0, 24.0),
            (-1840.0, -404.0)
        );
    }

    #[test]
    fn picker_clamps_to_small_work_areas() {
        let work = Rect {
            x: 0.0,
            y: 0.0,
            width: 500.0,
            height: 320.0,
        };
        let (width, height) = picker_window_size(work, 6);
        // The area cannot honour the floors (480 x 360), so the visible bounds win.
        assert_eq!(width, 452.0);
        assert_eq!(height, 272.0);
        assert!(width <= work.width);
        assert!(height <= work.height);
    }

    #[test]
    fn picker_widths_are_twenty_percent_wider_than_the_historical_targets() {
        let work = display(0.0, 0.0, 3000.0, 1600.0).work_area;
        for (count, historical) in [(1, 560.0), (2, 620.0), (3, 900.0), (4, 1120.0), (8, 1120.0)] {
            assert_eq!(
                picker_window_size(work, count).0,
                historical * 1.2,
                "{count} options must be 120% of the previous width"
            );
        }
    }

    #[test]
    fn picker_height_is_three_quarters_of_the_usable_height() {
        for usable in [760.0, 1055.0, 1440.0] {
            let work = Rect {
                x: 0.0,
                y: 25.0,
                width: 1920.0,
                height: usable,
            };
            let (_, height) = picker_window_size(work, 2);
            assert_eq!(height, usable * HEIGHT_RATIO, "usable height {usable}");
            assert_eq!(height, usable * 0.75);
        }
        // Below the floor the panel stays a usable size instead of shrinking away.
        let tiny = Rect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 440.0,
        };
        assert_eq!(picker_window_size(tiny, 2).1, MIN_HEIGHT);
    }

    #[test]
    fn picker_grows_with_option_count() {
        let work = display(0.0, 0.0, 3000.0, 1600.0);
        let one = picker_window_size(work.work_area, 1);
        let four = picker_window_size(work.work_area, 4);
        assert!(four.0 > one.0, "more cards get a wider panel");
        // Height is display-driven once the cards still fit on one row.
        assert_eq!(one.1, work.work_area.height * HEIGHT_RATIO);
        assert_eq!(four.1, one.1);
    }

    #[test]
    fn picker_adds_a_row_for_more_than_four_options() {
        // A 720pt screen puts the three-quarters target at 521pt: two card rows
        // (428pt content) fit, three rows (584pt) do not.
        let work = Rect {
            x: 0.0,
            y: 25.0,
            width: 1440.0,
            height: 695.0,
        };
        let four = picker_window_size(work, 4);
        let eight = picker_window_size(work, 8);
        let nine = picker_window_size(work, 9);
        assert_eq!(
            four.1,
            work.height * HEIGHT_RATIO,
            "one row fits inside the three-quarter-height target"
        );
        assert_eq!(
            eight.1,
            work.height * HEIGHT_RATIO,
            "two rows still fit inside the three-quarter-height target"
        );
        assert!(
            nine.1 > eight.1,
            "a third row must grow the panel instead of clipping"
        );
        assert_eq!(nine.1, CHROME_HEIGHT + 3.0 * ROW_HEIGHT);
        assert!(nine.1 <= work.height - AREA_INSET);
    }

    #[test]
    fn picker_rows_follow_the_live_column_count() {
        // A 700pt work area clamps the panel to 652pt, where the CSS grid uses two
        // columns instead of four, so the same option count needs more rows.
        let work = Rect {
            x: 0.0,
            y: 0.0,
            width: 700.0,
            height: 760.0,
        };
        let target = work.height * HEIGHT_RATIO;
        assert_eq!(
            picker_window_size(work, 2).0,
            652.0,
            "clamped to the work area minus the inset"
        );
        // One and two rows still fit inside the three-quarter-height target...
        assert_eq!(
            picker_window_size(work, 2).1,
            target,
            "two cards share one row"
        );
        assert_eq!(picker_window_size(work, 3).1, target);
        assert_eq!(picker_window_size(work, 4).1, target, "two rows fit");
        // ...three rows in two columns do not, so the panel grows instead of
        // clipping the last row behind the footer.
        let five = picker_window_size(work, 5).1;
        assert_eq!(five, CHROME_HEIGHT + 3.0 * ROW_HEIGHT);
        assert!(five > target, "five cards span three rows in two columns");
        assert!(five <= work.height - AREA_INSET);

        // Below 520pt the grid collapses to a single column, so every extra card
        // adds a row, and a fourth card hits the work-area clamp.
        let narrow = Rect {
            x: 0.0,
            y: 0.0,
            width: 500.0,
            height: 760.0,
        };
        assert!(
            narrow.width - AREA_INSET < MIN_WIDTH,
            "the panel is narrow enough for the single-column layout"
        );
        let one = picker_window_size(narrow, 1).1;
        let two = picker_window_size(narrow, 2).1;
        let three = picker_window_size(narrow, 3).1;
        let four = picker_window_size(narrow, 4).1;
        assert_eq!(one, narrow.height * HEIGHT_RATIO);
        assert_eq!(two, one, "two cards still fit inside the height target");
        assert_eq!(three, CHROME_HEIGHT + 3.0 * ROW_HEIGHT);
        assert_eq!(four, narrow.height - AREA_INSET, "clamped to the area");
        assert!(three > two, "one column means one row each");
    }

    #[test]
    fn picker_height_still_clamps_on_short_displays() {
        let work = Rect {
            x: 0.0,
            y: 0.0,
            width: 1600.0,
            height: 400.0,
        };
        let (_, height) = picker_window_size(work, 9);
        assert_eq!(height, 352.0, "clamped to the work area minus margins");
    }

    /// The panel must land fully inside the display it was sized for, including
    /// secondary screens with negative origins and very small work areas.
    #[test]
    fn picker_always_fits_inside_the_target_work_area() {
        let areas = [
            Rect {
                x: 0.0,
                y: 25.0,
                width: 1920.0,
                height: 1055.0,
            },
            Rect {
                x: -2560.0,
                y: -1440.0,
                width: 2560.0,
                height: 1415.0,
            },
            Rect {
                x: 1920.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
            },
            Rect {
                x: 0.0,
                y: 25.0,
                width: 3440.0,
                height: 1415.0,
            },
            Rect {
                x: 0.0,
                y: 25.0,
                width: 1366.0,
                height: 743.0,
            },
            Rect {
                x: -1600.0,
                y: -1080.0,
                width: 1920.0,
                height: 1055.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 480.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 420.0,
                height: 300.0,
            },
        ];
        for work in areas {
            for count in [1usize, 2, 3, 4, 6, 8, 9, 12] {
                let (width, height) = picker_window_size(work, count);
                let (x, y) = bottom_center_position(work, width, height, 24.0);
                assert!(
                    width > 0.0
                        && height > 0.0
                        && x >= work.x
                        && y >= work.y
                        && x + width <= work.x + work.width
                        && y + height <= work.y + work.height,
                    "count {count} on {work:?}: {x},{y} {width}x{height}"
                );
                // Centred horizontally on its own display.
                assert!(
                    (x + width / 2.0 - (work.x + work.width / 2.0)).abs() < 1e-9,
                    "count {count} on {work:?} is not centred: {x} + {width}/2"
                );
            }
        }
    }

    /// Ultrawide screens must not stretch the panel past the card-grid ceiling.
    #[test]
    fn picker_width_caps_on_ultrawide_displays() {
        let ultrawide = Rect {
            x: 0.0,
            y: 25.0,
            width: 3440.0,
            height: 1415.0,
        };
        assert_eq!(picker_window_size(ultrawide, 1).0, 560.0 * 1.2);
        // The four-plus card layout stops at MAX_WIDTH even though the area is
        // nearly 3.5k points wide.
        assert_eq!(picker_window_size(ultrawide, 4).0, MAX_WIDTH);
        assert_eq!(picker_window_size(ultrawide, 12).0, MAX_WIDTH);
        // And the height is still only three quarters of the usable area.
        assert_eq!(
            picker_window_size(ultrawide, 12).1,
            ultrawide.height * HEIGHT_RATIO
        );
    }

    /// Sessions on different monitors re-derive geometry from that monitor's own
    /// work area: bigger screen → bigger panel, back to the small screen → back
    /// to the small panel, with no dependence on what the previous session used.
    #[test]
    fn picker_resizes_when_sessions_move_between_monitors() {
        let small = Rect {
            x: 1920.0,
            y: 25.0,
            width: 1366.0,
            height: 743.0,
        };
        let standard = Rect {
            x: 0.0,
            y: 25.0,
            width: 1920.0,
            height: 1055.0,
        };
        let large = Rect {
            x: -2560.0,
            y: 25.0,
            width: 2560.0,
            height: 1415.0,
        };
        let ultrawide = Rect {
            x: 0.0,
            y: 25.0,
            width: 3440.0,
            height: 1415.0,
        };

        let geometry = |work: Rect, count: usize| {
            let (width, height) = picker_window_size(work, count);
            let (x, y) = bottom_center_position(work, width, height, 24.0);
            (x, y, width, height)
        };

        let small_first = geometry(small, 5);
        let standard_next = geometry(standard, 5);
        let large_next = geometry(large, 5);
        let ultrawide_next = geometry(ultrawide, 5);

        // Growing displays grow the panel (bounded by the width ceiling and the
        // three-quarters height target).
        assert!(standard_next.3 > small_first.3);
        assert!(large_next.3 > standard_next.3);
        assert_eq!(ultrawide_next.3, large_next.3, "same vertical size");
        assert!(ultrawide_next.2 >= large_next.2, "ultrawide gets width");
        assert!(ultrawide_next.2 <= MAX_WIDTH);

        // Returning to the small monitor returns to the small panel: the formula
        // is stateless, so a previous large session cannot leave stale dimensions.
        assert_eq!(geometry(small, 5), small_first);
        assert_eq!(geometry(standard, 5), standard_next);

        // Repeated sessions on different displays each land inside their own area.
        for (work, session) in [
            (small, small_first),
            (standard, standard_next),
            (large, large_next),
            (ultrawide, ultrawide_next),
        ] {
            let (x, y, width, height) = session;
            assert!(
                x >= work.x
                    && y >= work.y
                    && x + width <= work.x + work.width
                    && y + height <= work.y + work.height,
                "stale or oversized panel for {work:?}: {x},{y} {width}x{height}"
            );
        }
    }

    /// Three result cards must share one horizontal row on every normally
    /// supported monitor — MacBook built-in Retina and 1× externals alike —
    /// so a monitor switch can never silently split the `[原文][润色][翻译]`
    /// row. Columns come from the final clamped width, and the three-card
    /// target (1080pt) clears the two-column tier on all these displays.
    #[test]
    fn three_cards_stay_one_row_on_supported_monitors() {
        let monitors = [
            ("MacBook built-in 1440", 1440.0, 875.0),
            ("MacBook scaled 1280", 1280.0, 755.0),
            ("1366 laptop", 1366.0, 743.0),
            ("1080p external", 1920.0, 1055.0),
            ("1440p external", 2560.0, 1415.0),
            ("ultrawide", 3440.0, 1415.0),
        ];
        for (name, width, height) in monitors {
            let work = Rect {
                x: 0.0,
                y: 25.0,
                width,
                height,
            };
            let layout = picker_layout(work, 3);
            assert_eq!(layout.columns, 3, "{name}: three cards share one row");
            assert_eq!(layout.rows, 1, "{name}: one row, no stacking");
            assert!(
                layout.width <= width - AREA_INSET,
                "{name}: panel must fit inside the work area"
            );
        }
    }

    /// The layout depends on logical points only — never on the backing scale.
    /// Identical logical sizes on a Retina panel (2×) and a 1× display produce
    /// byte-identical geometry, so switching between displays with different
    /// scale factors can never half- or double-size the picker.
    #[test]
    fn layout_is_independent_of_backing_scale() {
        let retina = scaled_display(0.0, 25.0, 1440.0, 875.0, 2.0);
        let external = scaled_display(0.0, 25.0, 1440.0, 875.0, 1.0);
        for count in [1usize, 2, 3, 4, 6, 9] {
            assert_eq!(
                picker_layout(retina.work_area, count),
                picker_layout(external.work_area, count),
                "count {count}: scale must not enter the layout"
            );
        }
        // The scale factor is still carried through for diagnostics.
        assert_eq!(retina.scale_factor, 2.0);
        assert_eq!(external.scale_factor, 1.0);
    }

    /// Alternating sessions between the built-in Retina display and a 1×
    /// external each derive geometry fresh from their own target: the second
    /// and third visit to a display match the first visit exactly — no session
    /// inherits stale dimensions from the previously shown monitor — and every
    /// placement stays inside its own work area.
    #[test]
    fn alternating_monitor_sessions_recompute_from_scratch() {
        let builtin = scaled_display(0.0, 25.0, 1440.0, 875.0, 2.0);
        let external = scaled_display(1920.0, 25.0, 1920.0, 1055.0, 1.0);
        let sequence = [
            builtin.clone(),
            external.clone(),
            builtin.clone(),
            external.clone(),
            builtin.clone(),
        ];

        let layouts: Vec<PickerLayout> = sequence
            .iter()
            .map(|display| picker_layout(display.work_area, 3))
            .collect();

        assert_eq!(layouts[0], layouts[2], "built-in revisit must match");
        assert_eq!(layouts[2], layouts[4], "built-in revisit must match");
        assert_eq!(layouts[1], layouts[3], "external revisit must match");
        assert_ne!(
            layouts[0], layouts[1],
            "each display sizes its own panel (height differs)"
        );

        for (display, layout) in sequence.iter().zip(&layouts) {
            let (x, y) =
                bottom_center_position(display.work_area, layout.width, layout.height, 24.0);
            assert!(
                x >= display.work_area.x
                    && y >= display.work_area.y
                    && x + layout.width <= display.work_area.x + display.work_area.width
                    && y + layout.height <= display.work_area.y + display.work_area.height,
                "panel escapes {:?}: {x},{y} {}x{}",
                display.work_area,
                layout.width,
                layout.height
            );
        }
    }

    /// The explicit column policy: width tiers decide the grid, capped by the
    /// card count. The narrow single-column tier is the deliberate fallback for
    /// genuinely small displays, not something mid-range monitors can hit.
    #[test]
    fn column_policy_is_deterministic_per_width_tier() {
        assert_eq!(picker_columns(480.0, 3), 1, "narrow fallback tier");
        assert_eq!(picker_columns(519.0, 3), 1);
        assert_eq!(picker_columns(520.0, 3), 2);
        assert_eq!(picker_columns(652.0, 3), 2);
        assert_eq!(picker_columns(779.0, 3), 2);
        assert_eq!(
            picker_columns(780.0, 3),
            3,
            "full tier still capped by cards"
        );
        assert_eq!(picker_columns(1080.0, 3), 3);
        assert_eq!(picker_columns(1080.0, 4), 4);
        assert_eq!(picker_columns(1344.0, 6), 4, "capped at four columns");
        assert_eq!(
            picker_columns(1080.0, 0),
            1,
            "empty session keeps one column"
        );
    }

    /// The two placement helpers must describe the same physical rectangle: same
    /// x, and bottom-left origin y equal to `primary_height - (top-left y + h)`.
    /// Any drift between them would place the native window somewhere other than
    /// where the diagnostics claim it is.
    #[test]
    fn bl_and_tl_placements_describe_the_same_rect() {
        let areas = [
            Rect {
                x: 0.0,
                y: 25.0,
                width: 1920.0,
                height: 1055.0,
            },
            Rect {
                x: 1728.0,
                y: -531.0,
                width: 1920.0,
                height: 1080.0,
            },
            Rect {
                x: 1728.0,
                y: 549.0,
                width: 1920.0,
                height: 1080.0,
            },
            Rect {
                x: -2560.0,
                y: 0.0,
                width: 2560.0,
                height: 1415.0,
            },
            Rect {
                x: 0.0,
                y: 38.0,
                width: 1728.0,
                height: 1079.0,
            },
        ];
        for work in areas {
            let work_bl = mirror_vertical(work, TEST_PRIMARY_HEIGHT);
            for (width, height) in [(1080.0, 810.0), (672.0, 570.0), (452.0, 272.0)] {
                let (x, y) = bottom_center_position(work, width, height, 24.0);
                let (bx, by) = bottom_center_position_bl(work_bl, width, 24.0);
                assert_eq!(bx, x, "x must be identical in both spaces");
                assert!(
                    (by - (TEST_PRIMARY_HEIGHT - (y + height))).abs() < 1e-9,
                    "bottom edge mismatch for {work:?}: bl y {by} vs mirror {}",
                    TEST_PRIMARY_HEIGHT - (y + height)
                );
                // And the bottom-left placement really sits inside the mirrored area.
                assert!(
                    bx >= work_bl.x
                        && by >= work_bl.y
                        && bx + width <= work_bl.x + work_bl.width
                        && by + height <= work_bl.y + work_bl.height,
                    "bl containment failed for {work:?}"
                );
            }
        }
    }

    /// Fixture mirroring this repository's development Mac: a 2× Retina built-in
    /// primary (1728×1117pt, 1079pt visible) at origin, one external whose top
    /// sits 531pt ABOVE the primary top, and one whose top sits 549pt BELOW it.
    /// The bottom-left values are the raw `NSScreen.visibleFrame`s read off the
    /// live machine; the top-left values are what macOS `CGWindowBounds` for
    /// windows on those screens fall into. Pinning these together is the
    /// regression guard for the coordinate conversion itself — a wrong primary
    /// height or sign convention shifts every placement and is caught here.
    #[test]
    fn three_screen_layout_roundtrips_like_real_macos() {
        let primary_height = 1117.0;
        let builtin_bl = Rect {
            x: 0.0,
            y: 0.0,
            width: 1728.0,
            height: 1079.0,
        };
        let above_bl = Rect {
            x: 1728.0,
            y: 568.0,
            width: 1920.0,
            height: 1080.0,
        };
        let below_bl = Rect {
            x: 1728.0,
            y: -512.0,
            width: 1920.0,
            height: 1080.0,
        };

        assert_eq!(
            mirror_vertical(builtin_bl, primary_height),
            Rect {
                x: 0.0,
                y: 38.0,
                width: 1728.0,
                height: 1079.0
            }
        );
        assert_eq!(
            mirror_vertical(above_bl, primary_height),
            Rect {
                x: 1728.0,
                y: -531.0,
                width: 1920.0,
                height: 1080.0
            }
        );
        assert_eq!(
            mirror_vertical(below_bl, primary_height),
            Rect {
                x: 1728.0,
                y: 549.0,
                width: 1920.0,
                height: 1080.0
            }
        );
        // Mirroring is an involution: native placement computed from the raw
        // bottom-left data lands exactly where the top-left math predicts.
        for bl in [builtin_bl, above_bl, below_bl] {
            assert_eq!(
                mirror_vertical(mirror_vertical(bl, primary_height), primary_height),
                bl
            );
        }
    }

    /// Window bounds as macOS reports them (CG top-left global points, from live
    /// machines) must select the correct screen in this three-display layout —
    /// including a window straddling the primary and the upper external.
    #[test]
    fn real_world_window_bounds_select_the_owning_screen() {
        let primary_height = 1117.0;
        let mk = |name: &str, frame_bl: Rect, scale: f64| Display {
            name: name.to_string(),
            frame: mirror_vertical(frame_bl, primary_height),
            work_area: mirror_vertical(frame_bl, primary_height),
            work_area_bl: frame_bl,
            scale_factor: scale,
        };
        let displays = vec![
            mk(
                "builtin",
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1728.0,
                    height: 1117.0,
                },
                2.0,
            ),
            mk(
                "above",
                Rect {
                    x: 1728.0,
                    y: 568.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                1.0,
            ),
            mk(
                "below",
                Rect {
                    x: 1728.0,
                    y: -512.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                1.0,
            ),
        ];

        // Live Safari bounds on the built-in.
        assert_eq!(
            display_for_window(
                Rect {
                    x: 64.0,
                    y: 38.0,
                    width: 1664.0,
                    height: 1079.0
                },
                &displays
            ),
            Some(0)
        );
        // A window on the upper external (CG y negative, above the primary).
        assert_eq!(
            display_for_window(
                Rect {
                    x: 1799.0,
                    y: -506.0,
                    width: 1848.0,
                    height: 1055.0
                },
                &displays
            ),
            Some(1)
        );
        // A window on the lower external (CG y beyond the primary's bottom).
        assert_eq!(
            display_for_window(
                Rect {
                    x: 1799.0,
                    y: 574.0,
                    width: 1848.0,
                    height: 1055.0
                },
                &displays
            ),
            Some(2)
        );
        // Straddling the primary and the upper external: the external owns it.
        assert_eq!(
            display_for_window(
                Rect {
                    x: 1600.0,
                    y: 100.0,
                    width: 400.0,
                    height: 500.0
                },
                &displays
            ),
            Some(1)
        );
    }

    /// Repeated sessions across the real three-display layout each land inside
    /// their own screen in BOTH coordinate spaces — the exact pattern of the
    /// field bug (built-in → external → built-in → other external), evaluated on
    /// the geometry that the native `setFrame:` path consumes.
    #[test]
    fn alternating_real_displays_place_inside_both_spaces() {
        let primary_height = 1117.0;
        let screens = [
            (
                "builtin",
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1728.0,
                    height: 1079.0,
                },
            ),
            (
                "above",
                Rect {
                    x: 1728.0,
                    y: 568.0,
                    width: 1920.0,
                    height: 1080.0,
                },
            ),
            (
                "below",
                Rect {
                    x: 1728.0,
                    y: -512.0,
                    width: 1920.0,
                    height: 1080.0,
                },
            ),
        ];
        let visits = [0usize, 1, 0, 2, 1, 0];
        let mut results = Vec::new();
        for visit in visits {
            let (name, work_bl) = screens[visit];
            let work_tl = mirror_vertical(work_bl, primary_height);
            let layout = picker_layout(work_tl, 3);
            let (bx, by) = bottom_center_position_bl(work_bl, layout.width, 24.0);
            assert!(
                bx >= work_bl.x
                    && by >= work_bl.y
                    && bx + layout.width <= work_bl.x + work_bl.width + 1e-9
                    && by + layout.height <= work_bl.y + work_bl.height + 1e-9,
                "{name}: native frame escapes its screen"
            );
            results.push((layout.width, layout.height, bx, by));
        }
        // Each visit is a pure function of the visited screen: revisits match
        // byte-for-byte, and no session inherits another screen's geometry.
        assert_eq!(results[0], results[2]);
        assert_eq!(results[2], results[5]);
        assert_eq!(results[1], results[4]);
        assert_ne!(results[0], results[1]);
    }

    #[test]
    fn clamp_to_prefers_the_area_limit_over_the_floor() {
        assert_eq!(clamp_to(1000.0, 480.0, 1200.0), 1000.0);
        assert_eq!(clamp_to(100.0, 480.0, 1200.0), 480.0);
        assert_eq!(clamp_to(5000.0, 480.0, 1200.0), 1200.0);
        assert_eq!(clamp_to(5000.0, 480.0, 300.0), 300.0);
        assert_eq!(clamp_to(5000.0, 480.0, -40.0), 0.0);
    }
}
