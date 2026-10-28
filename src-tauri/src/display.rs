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
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Display {
    pub frame: Rect,
    pub work_area: Rect,
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

/// Panel sizing for the floating picker.
///
/// Every piece of picker geometry comes out of this one function so the native
/// window, the CSS grid and the tests cannot disagree:
///
/// * width  — 120% of the historical per-count width, clamped into the work area;
/// * height — half of the target display's usable height, grown only when the card
///   rows genuinely need more, and clamped into the work area.
///
/// Both axes keep a floor so the panel stays readable on small laptops and can
/// never exceed the visible area (`clamp_to` prefers the area limit over the
/// floor). The panel content itself stretches (`grid-auto-rows: minmax(…, 1fr)`),
/// so the extra space goes to the cards instead of empty gaps.
pub(crate) fn picker_window_size(work_area: Rect, option_count: usize) -> (f64, f64) {
    let base_width: f64 = match option_count {
        0 | 1 => 560.0,
        2 => 620.0,
        3 => 900.0,
        _ => 1120.0,
    };
    let width = clamp_to(
        base_width * WIDTH_GROWTH,
        MIN_WIDTH,
        work_area.width - AREA_INSET,
    );

    // Columns mirror `web/src/picker/PickerApp.tsx` (one column below 520px, two
    // below 780px, four above). Deriving them from the *clamped* width matters on
    // narrow displays: there the panel is narrower than its target, fits fewer
    // cards per row and therefore needs more rows of height.
    let max_columns = if width < 520.0 {
        1
    } else if width < 780.0 {
        2
    } else {
        4
    };
    let columns = max_columns.min(option_count.max(1));
    let rows = option_count.div_ceil(columns).max(1);

    let content_height = CHROME_HEIGHT + rows as f64 * ROW_HEIGHT;
    let height = clamp_to(
        (work_area.height * HEIGHT_RATIO).max(content_height),
        MIN_HEIGHT,
        work_area.height - AREA_INSET,
    );
    (width, height)
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
/// Target share of the display's usable height.
const HEIGHT_RATIO: f64 = 0.5;
/// Readability floors.
const MIN_WIDTH: f64 = 480.0;
const MIN_HEIGHT: f64 = 360.0;
/// Keep the panel at least this far from the work-area edges.
const AREA_INSET: f64 = 48.0;
/// One card row: CSS `grid-auto-rows` minimum (156px) plus the grid gap (8px).
const ROW_HEIGHT: f64 = 164.0;
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
    Display {
        frame,
        work_area: Rect {
            y: 25.0,
            height: 875.0,
            ..frame
        },
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

/// Resolve the display on the main thread. AppKit screen/workspace lookups are
/// safest there; the session start path runs on the async runtime.
pub(crate) async fn resolve_target_display_on_main(app: &tauri::AppHandle) -> TargetDisplay {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(resolve_target_display());
        })
        .is_err()
    {
        return resolve_target_display();
    }
    match rx.await {
        Ok(target) => target,
        Err(_) => {
            log_app!(
                warn,
                "Display capture was cancelled; using primary fallback"
            );
            TargetDisplay {
                display: fallback_display(),
                target: None,
            }
        }
    }
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
        let index = target
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
            })
            .unwrap_or(0);

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

            // Convert from AppKit's bottom-left origin to the top-left global
            // coordinate space used by CoreGraphics and Tauri logical positions.
            // The primary screen is the one at (0, 0); `screens` is ordered with
            // it first on every macOS version we support.
            let first: *mut AnyObject = msg_send![screens, objectAtIndex: 0usize];
            let first_frame: NSRect = msg_send![first, frame];
            let primary_height = first_frame.size.height;

            let mut result = Vec::with_capacity(count);
            for index in 0..count {
                let screen: *mut AnyObject = msg_send![screens, objectAtIndex: index];
                let frame: NSRect = msg_send![screen, frame];
                let visible: NSRect = msg_send![screen, visibleFrame];
                result.push(Display {
                    frame: to_top_left(frame, primary_height),
                    work_area: to_top_left(visible, primary_height),
                });
            }
            Some(result)
        }
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
        Display {
            frame: Rect {
                x,
                y,
                width,
                height,
            },
            // Real work areas exclude the menu bar; tests keep the same bounds
            // unless a test is specifically about positioning.
            work_area: Rect {
                x,
                y,
                width,
                height,
            },
        }
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
    fn picker_height_is_half_the_usable_height() {
        for usable in [760.0, 1055.0, 1440.0] {
            let work = Rect {
                x: 0.0,
                y: 25.0,
                width: 1920.0,
                height: usable,
            };
            let (_, height) = picker_window_size(work, 2);
            assert_eq!(height, usable * 0.5, "usable height {usable}");
        }
        // Below the floor the panel stays a usable size instead of shrinking away.
        let tiny = Rect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 700.0,
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
        let work = Rect {
            x: 0.0,
            y: 25.0,
            width: 1440.0,
            height: 900.0,
        };
        let four = picker_window_size(work, 4);
        let eight = picker_window_size(work, 8);
        let nine = picker_window_size(work, 9);
        assert_eq!(four.1, 450.0, "one row fits inside the half-height target");
        assert_eq!(
            eight.1, 450.0,
            "two rows still fit inside the half-height target"
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
            height: 1200.0,
        };
        let half = work.height * HEIGHT_RATIO;
        assert_eq!(
            picker_window_size(work, 2).0,
            652.0,
            "clamped to the work area minus the inset"
        );
        // One and two rows still fit inside the half-height target...
        assert_eq!(
            picker_window_size(work, 2).1,
            half,
            "two cards share one row"
        );
        assert_eq!(picker_window_size(work, 3).1, half);
        assert_eq!(picker_window_size(work, 4).1, half, "two rows fit");
        // ...three rows in two columns do not, so the panel grows instead of
        // clipping the last row behind the footer.
        let five = picker_window_size(work, 5).1;
        assert_eq!(five, CHROME_HEIGHT + 3.0 * ROW_HEIGHT);
        assert!(five > half, "five cards span three rows in two columns");
        assert!(five <= work.height - AREA_INSET);

        // Below 520pt the grid collapses to a single column, so every extra card
        // adds a row. A short display keeps the half-height target small enough for
        // that row growth to be the binding constraint.
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
        assert_eq!(one, narrow.height * HEIGHT_RATIO);
        assert_eq!(two, CHROME_HEIGHT + 2.0 * ROW_HEIGHT);
        assert_eq!(three, CHROME_HEIGHT + 3.0 * ROW_HEIGHT);
        assert!(three > two && two > one, "one column means one row each");
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

    #[test]
    fn clamp_to_prefers_the_area_limit_over_the_floor() {
        assert_eq!(clamp_to(1000.0, 480.0, 1200.0), 1000.0);
        assert_eq!(clamp_to(100.0, 480.0, 1200.0), 480.0);
        assert_eq!(clamp_to(5000.0, 480.0, 1200.0), 1200.0);
        assert_eq!(clamp_to(5000.0, 480.0, 300.0), 300.0);
        assert_eq!(clamp_to(5000.0, 480.0, -40.0), 0.0);
    }
}
