//! Reusable non-activating contextual WebView and CG-001 overlay controller.

use async_trait::async_trait;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
#[cfg(target_os = "macos")]
use tauri::{LogicalPosition, LogicalSize};
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use tauri::{PhysicalPosition, PhysicalSize};
use tokio::sync::oneshot;

#[cfg(not(target_os = "macos"))]
use crate::placement::place_overlay_on_monitors;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use crate::placement::MonitorWorkArea;
use crate::{
    contracts::{AppError, AppErrorCode, PhysicalRect, SelectionSnapshot, TranslationResult},
    placement::PhysicalSize as OverlaySize,
    platform::OverlayController,
};

const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_WIDTH: f64 = 380.0;
const OVERLAY_HEIGHT: f64 = 280.0;
/// The trigger window is kept to the pointer target so the transparent surface
/// does not cover — and suppress presses over — the document around it.
const TRIGGER_WIDTH: f64 = 44.0;
const TRIGGER_HEIGHT: f64 = 44.0;
const OVERLAY_GAP: f64 = 8.0;
const OVERLAY_IDLE_DESTROY_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectionEvent {
    request_id: u64,
    selection: SelectionSnapshot,
}

/// Pure readiness and idle-generation state for the contextual WebView.
#[derive(Debug, Default)]
pub(crate) struct OverlaySession {
    renderer_ready: bool,
    pending_selection: Option<SelectionSnapshot>,
    visible: bool,
    generation: u64,
}

impl OverlaySession {
    /// Presents immediately when ready, otherwise retains only the newest selection.
    pub(crate) fn present(&mut self, selection: SelectionSnapshot) -> Option<SelectionSnapshot> {
        self.generation = self.generation.wrapping_add(1);
        self.visible = true;
        if self.renderer_ready {
            Some(selection)
        } else {
            self.pending_selection = Some(selection);
            None
        }
    }

    /// Marks the listener ready and drains the one buffered first-use payload.
    pub(crate) fn renderer_ready(&mut self) -> Option<SelectionSnapshot> {
        self.renderer_ready = true;
        self.pending_selection.take()
    }

    /// Begins one cancellable idle-destruction generation.
    pub(crate) fn hide(&mut self) -> u64 {
        self.visible = false;
        self.pending_selection = None;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    /// Claims destruction only if no newer presentation invalidated the token.
    pub(crate) fn claim_idle_destruction(&mut self, token: u64) -> bool {
        if self.visible || self.generation != token {
            return false;
        }
        self.renderer_ready = false;
        self.pending_selection = None;
        self.generation = self.generation.wrapping_add(1);
        true
    }

    #[cfg(test)]
    pub(crate) fn is_renderer_ready(&self) -> bool {
        self.renderer_ready
    }
}

/// CG-001 overlay implementation backed by one lazily created Tauri WebView.
#[derive(Clone)]
pub struct TauriOverlayController {
    app: AppHandle,
    session: Arc<Mutex<OverlaySession>>,
    idle_cancel: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    operation: Arc<Mutex<()>>,
    #[cfg(target_os = "macos")]
    macos_anchor_logical: Arc<Mutex<Option<PhysicalRect>>>,
}

impl TauriOverlayController {
    /// Creates a controller for the application handle.
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            session: Arc::new(Mutex::new(OverlaySession::default())),
            idle_cancel: Arc::new(Mutex::new(None)),
            operation: Arc::new(Mutex::new(())),
            #[cfg(target_os = "macos")]
            macos_anchor_logical: Arc::new(Mutex::new(None)),
        }
    }

    fn window(&self) -> Result<WebviewWindow, AppError> {
        ensure_overlay(&self.app)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn record_selection_release(&self, position: crate::placement::PhysicalPoint) {
        if position.x.is_finite() && position.y.is_finite() {
            *self
                .macos_anchor_logical
                .lock()
                .expect("macOS overlay anchor") = Some(PhysicalRect {
                x: position.x,
                y: position.y,
                width: 1.0,
                height: 1.0,
            });
        }
    }

    /// Flushes the first buffered selection after the renderer listener exists.
    pub fn renderer_ready(&self) -> Result<(), AppError> {
        let _operation = self.operation.lock().expect("overlay operation");
        let pending = self
            .session
            .lock()
            .expect("overlay session")
            .renderer_ready();
        if let Some(selection) = pending {
            emit_selection(&self.app, selection)?;
        }
        Ok(())
    }

    /// Grows the trigger-sized window to the card footprint before content is
    /// emitted. Missing windows are ignored so a dismissed overlay stays hidden.
    fn expand_to_card(&self, _selection: &SelectionSnapshot) -> Result<(), AppError> {
        let _operation = self.operation.lock().expect("overlay operation");
        let Some(window) = self.app.get_webview_window(OVERLAY_LABEL) else {
            return Ok(());
        };
        #[cfg(target_os = "macos")]
        let anchor = self
            .macos_anchor_logical
            .lock()
            .expect("macOS overlay anchor")
            .as_ref()
            .copied()
            .ok_or_else(|| overlay_error("Overlay anchor is unavailable"))?;
        #[cfg(not(target_os = "macos"))]
        let anchor = _selection.anchor_physical_px;
        position_overlay(
            &window,
            anchor,
            OverlaySize {
                width: OVERLAY_WIDTH,
                height: OVERLAY_HEIGHT,
            },
        )
    }

    fn cancel_idle_destruction(&self) {
        if let Some(cancel) = self
            .idle_cancel
            .lock()
            .expect("overlay idle cancellation")
            .take()
        {
            let _ = cancel.send(());
        }
    }

    fn schedule_idle_destruction(&self, token: u64) {
        self.cancel_idle_destruction();
        let (cancel, cancelled) = oneshot::channel();
        *self.idle_cancel.lock().expect("overlay idle cancellation") = Some(cancel);
        let app = self.app.clone();
        let session = self.session.clone();
        let operation = self.operation.clone();
        tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(OVERLAY_IDLE_DESTROY_DELAY) => {
                    let _operation = operation.lock().expect("overlay operation");
                    let destroy = session
                        .lock()
                        .expect("overlay session")
                        .claim_idle_destruction(token);
                    if destroy {
                        if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
                            let _ = window.destroy();
                        }
                    }
                }
                _ = cancelled => {}
            }
        });
    }
}

#[async_trait]
impl OverlayController for TauriOverlayController {
    async fn show_button(&self, selection: &SelectionSnapshot) -> Result<(), AppError> {
        let _operation = self.operation.lock().expect("overlay operation");
        self.cancel_idle_destruction();
        let window = self.window()?;
        #[cfg(target_os = "macos")]
        let anchor = self
            .macos_anchor_logical
            .lock()
            .expect("macOS overlay anchor")
            .as_ref()
            .copied()
            .ok_or_else(|| overlay_error("Selection release position is unavailable"))?;
        #[cfg(not(target_os = "macos"))]
        let anchor = selection.anchor_physical_px;
        position_overlay(
            &window,
            anchor,
            OverlaySize {
                width: TRIGGER_WIDTH,
                height: TRIGGER_HEIGHT,
            },
        )?;
        let ready_selection = self
            .session
            .lock()
            .expect("overlay session")
            .present(selection.clone());
        window
            .show()
            .map_err(|_| overlay_error("Overlay could not be shown"))?;
        if let Some(selection) = ready_selection {
            emit_selection(&self.app, selection)?;
        }
        Ok(())
    }

    async fn show_loading(&self, selection: &SelectionSnapshot) -> Result<(), AppError> {
        self.expand_to_card(selection)?;
        self.app
            .emit_to(OVERLAY_LABEL, "translation-loading", ())
            .map_err(|_| overlay_error("Loading state could not be displayed"))
    }

    async fn show_result(
        &self,
        selection: &SelectionSnapshot,
        result: &TranslationResult,
    ) -> Result<(), AppError> {
        self.expand_to_card(selection)?;
        self.app
            .emit_to(OVERLAY_LABEL, "translation-result", result)
            .map_err(|_| overlay_error("Translation could not be displayed"))
    }

    async fn show_error(
        &self,
        selection: &SelectionSnapshot,
        error: &AppError,
    ) -> Result<(), AppError> {
        self.expand_to_card(selection)?;
        self.app
            .emit_to(OVERLAY_LABEL, "translation-error", error)
            .map_err(|_| overlay_error("Translation error could not be displayed"))
    }

    async fn hide(&self) -> Result<(), AppError> {
        let _operation = self.operation.lock().expect("overlay operation");
        hide_overlay(&self.app)?;
        let token = self.session.lock().expect("overlay session").hide();
        self.schedule_idle_destruction(token);
        Ok(())
    }
}

fn emit_selection(app: &AppHandle, selection: SelectionSnapshot) -> Result<(), AppError> {
    app.emit_to(
        OVERLAY_LABEL,
        "selection-resolved",
        SelectionEvent {
            request_id: selection.id,
            selection,
        },
    )
    .map_err(|_| overlay_error("Selection could not be displayed"))
}

/// Hides the contextual surface without destroying its reusable WebView.
pub fn hide_overlay(app: &AppHandle) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        window
            .hide()
            .map_err(|_| overlay_error("Overlay could not be hidden"))?;
    }
    Ok(())
}

/// Reports whether the current pointer is inside the visible contextual surface.
#[cfg(target_os = "macos")]
pub(crate) fn cursor_is_over_overlay(
    app: &AppHandle,
    cursor: crate::placement::PhysicalPoint,
) -> bool {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        return false;
    };
    if window.is_visible().ok() != Some(true) {
        return false;
    }
    let Ok(origin) = window.outer_position() else {
        return false;
    };
    let Ok(size) = window.outer_size() else {
        return false;
    };
    let Ok(scale_factor) = window.scale_factor() else {
        return false;
    };
    let Some(bounds) = crate::platform::macos::window::logical_window_bounds(
        crate::placement::PhysicalPoint {
            x: origin.x as f64,
            y: origin.y as f64,
        },
        OverlaySize {
            width: size.width as f64,
            height: size.height as f64,
        },
        scale_factor,
    ) else {
        return false;
    };
    crate::platform::macos::window::point_is_inside_overlay(cursor, bounds)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn cursor_is_over_overlay(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        return false;
    };
    if window.is_visible().ok() != Some(true) {
        return false;
    }
    let Ok(origin) = window.outer_position() else {
        return false;
    };
    let Ok(size) = window.outer_size() else {
        return false;
    };
    let Ok(cursor) = app.cursor_position() else {
        return false;
    };
    cursor.x >= origin.x as f64
        && cursor.x < origin.x as f64 + size.width as f64
        && cursor.y >= origin.y as f64
        && cursor.y < origin.y as f64 + size.height as f64
}

fn ensure_overlay(app: &AppHandle) -> Result<WebviewWindow, AppError> {
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        WebviewUrl::App("index.html?mode=overlay".into()),
    )
    .title("Translate selection")
    .inner_size(TRIGGER_WIDTH, TRIGGER_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focusable(false)
    .visible(false)
    .build()
    .map_err(|_| overlay_error("Overlay window could not be created"))?;

    #[cfg(target_os = "windows")]
    configure_native_nonactivation(&window)?;
    Ok(window)
}

#[cfg(target_os = "windows")]
fn position_overlay(
    window: &WebviewWindow,
    anchor: PhysicalRect,
    logical_size: OverlaySize,
) -> Result<(), AppError> {
    use crate::platform::windows::window::{
        monitor_work_area_for, position_non_activating_tool_window,
    };

    let work_area = monitor_work_area_for(anchor)?;
    let placement = place_overlay_on_monitors(anchor, &[work_area], logical_size, OVERLAY_GAP)
        .ok_or_else(|| overlay_error("Overlay position could not be resolved"))?;
    let hwnd = window
        .hwnd()
        .map_err(|_| overlay_error("Native overlay handle is unavailable"))?;
    position_non_activating_tool_window(hwnd, &placement)
}

#[cfg(target_os = "macos")]
fn position_overlay(
    window: &WebviewWindow,
    anchor_logical: PhysicalRect,
    logical_size: OverlaySize,
) -> Result<(), AppError> {
    use crate::platform::macos::{active_display_transforms, MacScreenGeometry};

    let screens: Vec<_> = active_display_transforms()
        .into_iter()
        .enumerate()
        .map(|(index, display)| MacScreenGeometry {
            id: format!("display-{index}"),
            logical_bounds: display.logical_bounds,
        })
        .collect();
    let placement = crate::platform::macos::window::place_overlay_in_screen_points(
        anchor_logical,
        &screens,
        logical_size,
        OVERLAY_GAP,
    )
    .ok_or_else(|| overlay_error("Overlay position could not be resolved"))?;

    window
        .set_size(LogicalSize::new(
            placement.size_logical_points.width,
            placement.size_logical_points.height,
        ))
        .and_then(|_| {
            window.set_position(LogicalPosition::new(
                placement.position_logical_points.x,
                placement.position_logical_points.y,
            ))
        })
        .map_err(|_| overlay_error("Overlay position could not be applied"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn position_overlay(
    window: &WebviewWindow,
    anchor: PhysicalRect,
    logical_size: OverlaySize,
) -> Result<(), AppError> {
    let monitors = window
        .available_monitors()
        .map_err(|_| overlay_error("Monitor topology is unavailable"))?;
    let work_areas: Vec<_> = monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| MonitorWorkArea {
            id: format!("monitor-{index}"),
            work_area_physical_px: PhysicalRect {
                x: monitor.position().x as f64,
                y: monitor.position().y as f64,
                width: monitor.size().width as f64,
                height: monitor.size().height as f64,
            },
            scale_factor: monitor.scale_factor(),
        })
        .collect();
    let placement = place_overlay_on_monitors(anchor, &work_areas, logical_size, OVERLAY_GAP)
        .ok_or_else(|| overlay_error("Overlay position could not be resolved"))?;

    window
        .set_size(PhysicalSize::new(
            placement.size_physical_px.width.round() as u32,
            placement.size_physical_px.height.round() as u32,
        ))
        .and_then(|_| {
            window.set_position(PhysicalPosition::new(
                placement.position_physical_px.x.round() as i32,
                placement.position_physical_px.y.round() as i32,
            ))
        })
        .map_err(|_| overlay_error("Overlay position could not be applied"))
}

#[cfg(target_os = "windows")]
fn configure_native_nonactivation(window: &WebviewWindow) -> Result<(), AppError> {
    use crate::platform::windows::apply_non_activating_tool_window;

    let hwnd = window
        .hwnd()
        .map_err(|_| overlay_error("Native overlay handle is unavailable"))?;
    apply_non_activating_tool_window(hwnd)
}

fn overlay_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}
