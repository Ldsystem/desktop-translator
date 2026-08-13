//! Tray-anchored panel for translating operator-typed text.

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

use crate::contracts::{AppError, AppErrorCode};

/// Window label for the tray panel.
pub const QUICK_LABEL: &str = "quick";
const QUICK_WIDTH: f64 = 400.0;
/// Tall enough for the composer plus a resolved translation without clipping.
const QUICK_HEIGHT: f64 = 470.0;
/// Clearance between the menu bar and the panel.
const QUICK_GAP: f64 = 6.0;

/// Shows the panel when hidden and hides it when already visible.
pub fn toggle(app: &AppHandle, anchor: PhysicalPosition<f64>) -> Result<(), AppError> {
    let visible = app
        .get_webview_window(QUICK_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    if visible {
        return hide(app);
    }
    show(app, anchor)
}

/// Presents the panel below the tray icon and gives it keyboard focus.
pub fn show(app: &AppHandle, anchor: PhysicalPosition<f64>) -> Result<(), AppError> {
    let window = ensure_window(app)?;
    position_panel(&window, anchor)?;
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|_| quick_error("Quick translation panel could not be shown"))?;
    // Lets the renderer clear stale text and focus the input on each open.
    app.emit_to(QUICK_LABEL, "quick-translate-opened", ())
        .map_err(|_| quick_error("Quick translation panel could not be prepared"))
}

/// Hides the panel without destroying its reusable WebView.
pub fn hide(app: &AppHandle) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(QUICK_LABEL) {
        window
            .hide()
            .map_err(|_| quick_error("Quick translation panel could not be hidden"))?;
    }
    Ok(())
}

fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, AppError> {
    if let Some(window) = app.get_webview_window(QUICK_LABEL) {
        return Ok(window);
    }

    WebviewWindowBuilder::new(
        app,
        QUICK_LABEL,
        WebviewUrl::App("index.html?mode=quick".into()),
    )
    .title("Quick Translate")
    .inner_size(QUICK_WIDTH, QUICK_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    // The native shadow is drawn around the square window, not the rounded
    // panel, so the surface renders its own shadow instead.
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|_| quick_error("Quick translation panel could not be created"))
}

/// Centers the panel under the tray anchor, clamped to the anchored monitor.
fn position_panel(window: &WebviewWindow, anchor: PhysicalPosition<f64>) -> Result<(), AppError> {
    let size = window
        .outer_size()
        .map_err(|_| quick_error("Quick translation panel size is unavailable"))?;
    let monitor = window
        .monitor_from_point(anchor.x, anchor.y)
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(|| quick_error("Monitor topology is unavailable"))?;

    let scale = monitor.scale_factor();
    let area_x = monitor.position().x as f64;
    let area_y = monitor.position().y as f64;
    let area_right = area_x + monitor.size().width as f64;
    let width = size.width as f64;
    let gap = QUICK_GAP * scale;

    let x = (anchor.x - width / 2.0).clamp(area_x, (area_right - width).max(area_x));
    let y = (anchor.y + gap).max(area_y);

    window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .map_err(|_| quick_error("Quick translation panel could not be positioned"))
}

fn quick_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}
