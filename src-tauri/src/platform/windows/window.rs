//! Non-activating tool-window policy and monitor work-area discovery.

use std::{ffi::c_void, sync::Arc};

use async_trait::async_trait;
use windows::Win32::{
    Foundation::{GetLastError, SetLastError, HWND, RECT, WIN32_ERROR},
    Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    },
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
            SET_WINDOW_POS_FLAGS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
            SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
            WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        },
    },
};

use crate::{
    contracts::{AppError, AppErrorCode, PhysicalRect, SelectionSnapshot, TranslationResult},
    placement::{place_overlay_on_monitors, MonitorWorkArea, PhysicalSize},
    platform::OverlayController,
};

/// Adds styles that keep contextual surfaces out of task switching and prevent
/// them from taking foreground activation.
pub const fn non_activating_tool_styles(base: WINDOW_EX_STYLE) -> WINDOW_EX_STYLE {
    WINDOW_EX_STYLE((base.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0)
}

/// Applies the non-activating tool-window policy to an existing native window.
pub fn apply_non_activating_tool_window(hwnd: HWND) -> Result<(), AppError> {
    // SAFETY: `hwnd` is supplied by the window owner and remains valid for this
    // synchronous style update. No borrowed pointers cross the API boundary.
    unsafe {
        let current = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
        let desired = non_activating_tool_styles(current);
        SetLastError(WIN32_ERROR(0));
        let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired.0 as isize);
        if previous == 0 && GetLastError() != WIN32_ERROR(0) {
            return Err(internal("could not apply non-activating window styles"));
        }

        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SET_WINDOW_POS_FLAGS(
                SWP_FRAMECHANGED.0
                    | SWP_NOACTIVATE.0
                    | SWP_NOMOVE.0
                    | SWP_NOOWNERZORDER.0
                    | SWP_NOSIZE.0
                    | SWP_NOZORDER.0,
            ),
        )
        .map_err(|_| internal("could not commit non-activating window styles"))?;
    }
    Ok(())
}

/// Shows a configured contextual surface without changing foreground focus.
pub fn show_without_activation(hwnd: HWND) {
    // SAFETY: ShowWindow uses only the caller-owned HWND and does not retain
    // pointers. SW_SHOWNOACTIVATE preserves the current foreground window.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

/// Content state delivered to the owner of the native overlay WebView.
#[derive(Debug, Clone)]
pub enum WindowsOverlayContent {
    Hidden,
    Button {
        selection: SelectionSnapshot,
    },
    Loading {
        selection: SelectionSnapshot,
    },
    Result {
        selection: SelectionSnapshot,
        result: TranslationResult,
    },
    Error {
        selection: SelectionSnapshot,
        error: AppError,
    },
}

/// Bridges native overlay state into the owning WebView/event layer.
pub trait WindowsOverlayContentDispatcher: Send + Sync {
    fn dispatch(&self, content: WindowsOverlayContent) -> Result<(), AppError>;
}

impl<F> WindowsOverlayContentDispatcher for F
where
    F: Fn(WindowsOverlayContent) -> Result<(), AppError> + Send + Sync,
{
    fn dispatch(&self, content: WindowsOverlayContent) -> Result<(), AppError> {
        self(content)
    }
}

/// Logical dimensions used for contextual button and popup placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowsOverlayMetrics {
    pub button_size: PhysicalSize,
    pub popup_size: PhysicalSize,
    pub gap: f64,
}

impl Default for WindowsOverlayMetrics {
    fn default() -> Self {
        Self {
            button_size: PhysicalSize {
                width: 36.0,
                height: 36.0,
            },
            popup_size: PhysicalSize {
                width: 360.0,
                height: 240.0,
            },
            gap: 8.0,
        }
    }
}

/// Concrete controller for one reusable, topmost, non-activating native window.
///
/// The HWND is stored as an integer so this controller can satisfy the
/// cross-thread adapter contract. The owner must keep the HWND alive until all
/// clones of this controller are dropped and marshal calls as required by its
/// windowing framework.
#[derive(Clone)]
pub struct WindowsOverlayWindow {
    hwnd: isize,
    metrics: WindowsOverlayMetrics,
    content: Arc<dyn WindowsOverlayContentDispatcher>,
}

impl WindowsOverlayWindow {
    /// Wraps an existing overlay HWND and immediately applies safe styles.
    pub fn new(
        hwnd: HWND,
        metrics: WindowsOverlayMetrics,
        content: Arc<dyn WindowsOverlayContentDispatcher>,
    ) -> Result<Self, AppError> {
        validate_overlay_metrics(metrics)?;
        apply_non_activating_tool_window(hwnd)?;
        Ok(Self {
            hwnd: hwnd.0 as isize,
            metrics,
            content,
        })
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    fn show(
        &self,
        selection: &SelectionSnapshot,
        logical_size: PhysicalSize,
        content: WindowsOverlayContent,
    ) -> Result<(), AppError> {
        self.content.dispatch(content)?;
        let monitor = monitor_work_area_for(selection.anchor_physical_px)?;
        let placement = place_overlay_on_monitors(
            selection.anchor_physical_px,
            &[monitor],
            logical_size,
            self.metrics.gap,
        )
        .ok_or_else(|| internal("could not place Windows overlay"))?;
        let x = finite_i32(placement.position_physical_px.x)?;
        let y = finite_i32(placement.position_physical_px.y)?;
        let width = positive_i32(placement.size_physical_px.width)?;
        let height = positive_i32(placement.size_physical_px.height)?;

        // SAFETY: the owner guarantees HWND lifetime. HWND_TOPMOST plus
        // SWP_NOACTIVATE displays above ordinary windows without changing the
        // foreground window or source selection.
        unsafe {
            SetWindowPos(
                self.hwnd(),
                Some(hwnd_topmost()),
                x,
                y,
                width,
                height,
                SET_WINDOW_POS_FLAGS(SWP_NOACTIVATE.0 | SWP_NOOWNERZORDER.0 | SWP_SHOWWINDOW.0),
            )
            .map_err(|_| internal("could not show topmost Windows overlay"))?;
        }
        Ok(())
    }
}

#[async_trait]
impl OverlayController for WindowsOverlayWindow {
    async fn show_button(&self, selection: &SelectionSnapshot) -> Result<(), AppError> {
        self.show(
            selection,
            self.metrics.button_size,
            WindowsOverlayContent::Button {
                selection: selection.clone(),
            },
        )
    }

    async fn show_loading(&self, selection: &SelectionSnapshot) -> Result<(), AppError> {
        self.show(
            selection,
            self.metrics.popup_size,
            WindowsOverlayContent::Loading {
                selection: selection.clone(),
            },
        )
    }

    async fn show_result(
        &self,
        selection: &SelectionSnapshot,
        result: &TranslationResult,
    ) -> Result<(), AppError> {
        self.show(
            selection,
            self.metrics.popup_size,
            WindowsOverlayContent::Result {
                selection: selection.clone(),
                result: result.clone(),
            },
        )
    }

    async fn show_error(
        &self,
        selection: &SelectionSnapshot,
        error: &AppError,
    ) -> Result<(), AppError> {
        self.show(
            selection,
            self.metrics.popup_size,
            WindowsOverlayContent::Error {
                selection: selection.clone(),
                error: error.clone(),
            },
        )
    }

    async fn hide(&self) -> Result<(), AppError> {
        // SAFETY: the owner guarantees HWND lifetime; SW_HIDE does not activate
        // another application window.
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_HIDE);
        }
        self.content.dispatch(WindowsOverlayContent::Hidden)
    }
}

fn hwnd_topmost() -> HWND {
    HWND(-1isize as *mut c_void)
}

/// Returns the physical work area and effective scale of the nearest monitor.
pub fn monitor_work_area_for(anchor: PhysicalRect) -> Result<MonitorWorkArea, AppError> {
    let native_rect = RECT {
        left: finite_i32(anchor.x)?,
        top: finite_i32(anchor.y)?,
        right: finite_i32(anchor.x + anchor.width)?,
        bottom: finite_i32(anchor.y + anchor.height)?,
    };

    // SAFETY: `native_rect` and `monitor_info` are initialized local storage.
    // MONITORINFOEXW's cbSize selects the extended device-name structure.
    unsafe {
        let monitor = MonitorFromRect(&native_rect, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return Err(internal("could not find a monitor for selection"));
        }

        let mut monitor_info = MONITORINFOEXW::default();
        monitor_info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(
            monitor,
            (&mut monitor_info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
        )
        .as_bool()
        {
            return Err(internal("could not read monitor work area"));
        }

        let mut dpi_x = 96;
        let mut dpi_y = 96;
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y)
            .map_err(|_| internal("could not read monitor scale"))?;

        let work = monitor_info.monitorInfo.rcWork;
        let device_len = monitor_info
            .szDevice
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(monitor_info.szDevice.len());

        Ok(MonitorWorkArea {
            id: String::from_utf16_lossy(&monitor_info.szDevice[..device_len]),
            work_area_physical_px: PhysicalRect {
                x: f64::from(work.left),
                y: f64::from(work.top),
                width: f64::from(work.right - work.left),
                height: f64::from(work.bottom - work.top),
            },
            scale_factor: f64::from(dpi_x) / 96.0,
        })
    }
}

fn finite_i32(value: f64) -> Result<i32, AppError> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        Err(internal(
            "selection geometry is outside Windows coordinates",
        ))
    } else {
        Ok(value.round() as i32)
    }
}

fn positive_i32(value: f64) -> Result<i32, AppError> {
    let value = finite_i32(value)?;
    if value <= 0 {
        Err(internal("Windows overlay size must be positive"))
    } else {
        Ok(value)
    }
}

fn validate_overlay_metrics(metrics: WindowsOverlayMetrics) -> Result<(), AppError> {
    for value in [
        metrics.button_size.width,
        metrics.button_size.height,
        metrics.popup_size.width,
        metrics.popup_size.height,
    ] {
        positive_i32(value)?;
    }
    if !metrics.gap.is_finite() || metrics.gap < 0.0 {
        return Err(internal(
            "Windows overlay gap must be finite and nonnegative",
        ));
    }
    Ok(())
}

fn internal(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::{
        WINDOW_EX_STYLE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    use super::{
        finite_i32, hwnd_topmost, non_activating_tool_styles, positive_i32,
        validate_overlay_metrics, WindowsOverlayMetrics,
    };

    #[test]
    fn policy_clears_appwindow_and_adds_required_flags() {
        let styles = non_activating_tool_styles(WS_EX_APPWINDOW);
        assert_eq!(styles.0 & WS_EX_APPWINDOW.0, 0);
        assert_ne!(styles.0 & WS_EX_NOACTIVATE.0, 0);
        assert_ne!(styles.0 & WS_EX_TOOLWINDOW.0, 0);
    }

    #[test]
    fn coordinate_conversion_rejects_non_finite_values() {
        assert!(finite_i32(f64::NAN).is_err());
        assert_eq!(finite_i32(-10.4).expect("finite coordinate"), -10);
        assert_eq!(
            non_activating_tool_styles(WINDOW_EX_STYLE(0)).0,
            0x0800_0080
        );
    }

    #[test]
    fn overlay_metrics_are_positive_and_monitor_scalable() {
        let metrics = WindowsOverlayMetrics::default();
        assert_eq!(positive_i32(metrics.button_size.width), Ok(36));
        assert_eq!(positive_i32(metrics.popup_size.height), Ok(240));
        assert!(positive_i32(0.0).is_err());
        assert_eq!(validate_overlay_metrics(metrics), Ok(()));
    }

    #[test]
    fn topmost_sentinel_preserves_signed_pointer_value() {
        assert_eq!(hwnd_topmost().0 as isize, -1);
    }
}
