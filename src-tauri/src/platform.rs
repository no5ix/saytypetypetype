//! Platform integration: macOS Dock visibility, app activation, and the
//! settings window show/rebuild lifecycle.

use tauri::AppHandle;
use tauri::Manager;

/// Hide the macOS menu bar extra while retaining the Dock application icon.
/// Regular activation policy keeps the app eligible for the Dock and normal
/// windows; unlike Accessory, it does not suppress the Dock icon.
#[cfg(target_os = "macos")]
pub(crate) fn use_dock_only() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Regular);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn use_dock_only() {}

/// Keep the app in the Dock. The menu bar extra is intentionally not used.
#[cfg(target_os = "macos")]
pub(crate) fn set_dock_visible(_visible: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        // Dock-only mode always uses Regular; never switch to Accessory because
        // that would remove the Dock icon when the settings window closes.
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn set_dock_visible(_visible: bool) {}

/// Bring the app to the foreground on macOS. Explicit activation ensures the
/// Dock-launched or reopened settings window is raised above other applications.
///
/// Uses the legacy `-activateIgnoringOtherApps:` deliberately: its replacement
/// `-[NSApplication activate]` only exists on macOS 14+, while this app targets
/// 10.15 (`minimumSystemVersion`), where the new selector is absent and would
/// crash at runtime. The `#[allow(deprecated)]` silences the SDK deprecation
/// notice for this compatibility requirement.
#[cfg(target_os = "macos")]
pub(crate) fn activate_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn activate_app() {}

/// Bring the settings window to front. If it was closed
/// (lazy lifecycle: closing destroys it to free the WebView), rebuild it from the
/// bundled window config before showing.
pub(crate) fn show_settings(app: &AppHandle) {
    // Retain the Dock-only activation policy before showing/focusing the window.
    use_dock_only();

    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    } else if let Some(cfg) = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "settings")
    {
        match tauri::WebviewWindowBuilder::from_config(app, cfg) {
            Ok(builder) => {
                if let Err(e) = builder.build() {
                    log_tray!(error, "failed to rebuild settings window: {e}");
                }
            }
            Err(e) => log_tray!(error, "failed to create settings window builder: {e}"),
        }
        // The rebuilt window has no implicit focus, so focus it explicitly.
        if let Some(window) = app.get_webview_window("settings") {
            let _ = window.set_focus();
        }
    }

    // Bring the app itself to the foreground so the window actually lands in front.
    activate_app();
}
