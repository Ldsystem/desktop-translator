//! AppKit policy for reusable, non-activating overlay panels.

use std::{
    ffi::{c_char, c_void, CString},
    mem,
    sync::Arc,
};

use async_trait::async_trait;

use crate::{
    contracts::{AppError, PhysicalRect, SelectionSnapshot, TranslationResult},
    placement::{PhysicalPoint, PhysicalSize},
    platform::OverlayController,
};

type Id = *mut c_void;
type Sel = *mut c_void;
type NSInteger = isize;
type NSUInteger = usize;
type ObjcBool = i8;

const NO: ObjcBool = 0;
const YES: ObjcBool = 1;
const NONACTIVATING_PANEL_MASK: NSUInteger = 1 << 7;
const CAN_JOIN_ALL_SPACES: NSUInteger = 1 << 0;
const IGNORES_CYCLE: NSUInteger = 1 << 6;
const FULL_SCREEN_AUXILIARY: NSUInteger = 1 << 8;
const STATUS_WINDOW_LEVEL: NSInteger = 25;

/// One macOS display in Quartz's global top-left logical-point space.
#[derive(Debug, Clone, PartialEq)]
pub struct MacScreenGeometry {
    pub id: String,
    pub logical_bounds: PhysicalRect,
}

/// Resolved macOS overlay geometry, expressed entirely in logical screen points.
#[derive(Debug, Clone, PartialEq)]
pub struct MacOverlayPlacement {
    pub screen_id: String,
    pub position_logical_points: PhysicalPoint,
    pub size_logical_points: PhysicalSize,
}

impl MacOverlayPlacement {
    pub fn bounds(&self) -> PhysicalRect {
        PhysicalRect {
            x: self.position_logical_points.x,
            y: self.position_logical_points.y,
            width: self.size_logical_points.width,
            height: self.size_logical_points.height,
        }
    }
}

/// Places an overlay without mixing Quartz logical points and backing pixels.
pub fn place_overlay_in_screen_points(
    anchor: PhysicalRect,
    screens: &[MacScreenGeometry],
    logical_size: PhysicalSize,
    gap: f64,
) -> Option<MacOverlayPlacement> {
    if !valid_rect(anchor) || !valid_size(logical_size) || !gap.is_finite() || gap < 0.0 {
        return None;
    }
    let center_x = anchor.x + anchor.width / 2.0;
    let center_y = anchor.y + anchor.height / 2.0;
    let screen =
        screens
            .iter()
            .filter(|screen| valid_rect(screen.logical_bounds))
            .min_by(|left, right| {
                distance_to_rect(center_x, center_y, left.logical_bounds)
                    .total_cmp(&distance_to_rect(center_x, center_y, right.logical_bounds))
            })?;
    let area = screen.logical_bounds;
    let right = area.x + area.width;
    let bottom = area.y + area.height;
    let mut x = anchor.x + anchor.width + gap;
    let mut y = anchor.y + anchor.height + gap;
    if x + logical_size.width > right {
        x = anchor.x - logical_size.width - gap;
    }
    if y + logical_size.height > bottom {
        y = anchor.y - logical_size.height - gap;
    }
    x = x.clamp(area.x, (right - logical_size.width).max(area.x));
    y = y.clamp(area.y, (bottom - logical_size.height).max(area.y));

    Some(MacOverlayPlacement {
        screen_id: screen.id.clone(),
        position_logical_points: PhysicalPoint { x, y },
        size_logical_points: logical_size,
    })
}

pub fn point_is_inside_overlay(point: PhysicalPoint, bounds: PhysicalRect) -> bool {
    point.x >= bounds.x
        && point.x < bounds.x + bounds.width
        && point.y >= bounds.y
        && point.y < bounds.y + bounds.height
}

/// Converts Tauri's backing-pixel window bounds back to Quartz logical points
/// using the overlay's current display scale.
pub fn logical_window_bounds(
    origin_backing_px: PhysicalPoint,
    size_backing_px: PhysicalSize,
    scale_factor: f64,
) -> Option<PhysicalRect> {
    if !valid_size(size_backing_px) || !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    Some(PhysicalRect {
        x: origin_backing_px.x / scale_factor,
        y: origin_backing_px.y / scale_factor,
        width: size_backing_px.width / scale_factor,
        height: size_backing_px.height / scale_factor,
    })
}

fn valid_rect(rect: PhysicalRect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
}

fn valid_size(size: PhysicalSize) -> bool {
    size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0
}

fn distance_to_rect(x: f64, y: f64, rect: PhysicalRect) -> f64 {
    let dx = if x < rect.x {
        rect.x - x
    } else if x > rect.x + rect.width {
        x - (rect.x + rect.width)
    } else {
        0.0
    };
    let dy = if y < rect.y {
        rect.y - y
    } else if y > rect.y + rect.height {
        y - (rect.y + rect.height)
    } else {
        0.0
    };
    dx * dx + dy * dy
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "objc")]
unsafe extern "C" {
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
}

/// Declarative policy, kept pure so convergence code can inspect and test the
/// exact AppKit behavior before handing over a native NSPanel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonActivatingPanelPolicy {
    pub style_mask: NSUInteger,
    pub collection_behavior: NSUInteger,
    pub level: NSInteger,
    pub opaque: bool,
    pub has_shadow: bool,
    pub hides_on_deactivate: bool,
    pub released_when_closed: bool,
    pub becomes_key_only_if_needed: bool,
}

impl Default for NonActivatingPanelPolicy {
    fn default() -> Self {
        Self {
            style_mask: NONACTIVATING_PANEL_MASK,
            collection_behavior: CAN_JOIN_ALL_SPACES | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY,
            level: STATUS_WINDOW_LEVEL,
            opaque: false,
            has_shadow: true,
            hides_on_deactivate: false,
            released_when_closed: false,
            becomes_key_only_if_needed: true,
        }
    }
}

/// State transitions that the native panel integration must render.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayCommand {
    ShowButton {
        selection: SelectionSnapshot,
    },
    ShowLoading {
        selection: SelectionSnapshot,
    },
    ShowResult {
        selection: SelectionSnapshot,
        result: TranslationResult,
    },
    ShowError {
        selection: SelectionSnapshot,
        error: AppError,
    },
    Hide,
}

type OverlayDispatcher = dyn Fn(OverlayCommand) -> Result<(), AppError> + Send + Sync + 'static;

/// Concrete `OverlayController` backed by a narrow native-panel dispatcher.
///
/// The dispatcher is the only integration seam: it must enqueue the command on
/// AppKit's main thread, reuse one configured NSPanel, and return promptly. This
/// keeps Tauri/window ownership outside the platform adapter while providing a
/// complete, mockable controller surface.
pub struct MacOverlayWindow {
    dispatch: Arc<OverlayDispatcher>,
}

impl MacOverlayWindow {
    pub fn new(
        dispatch: impl Fn(OverlayCommand) -> Result<(), AppError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            dispatch: Arc::new(dispatch),
        }
    }

    fn dispatch(&self, command: OverlayCommand) -> Result<(), AppError> {
        (self.dispatch)(command)
    }
}

#[async_trait]
impl OverlayController for MacOverlayWindow {
    async fn show_button(&self, selection: &SelectionSnapshot) -> Result<(), AppError> {
        self.dispatch(OverlayCommand::ShowButton {
            selection: selection.clone(),
        })
    }

    async fn show_loading(&self, selection: &SelectionSnapshot) -> Result<(), AppError> {
        self.dispatch(OverlayCommand::ShowLoading {
            selection: selection.clone(),
        })
    }

    async fn show_result(
        &self,
        selection: &SelectionSnapshot,
        result: &TranslationResult,
    ) -> Result<(), AppError> {
        self.dispatch(OverlayCommand::ShowResult {
            selection: selection.clone(),
            result: result.clone(),
        })
    }

    async fn show_error(
        &self,
        selection: &SelectionSnapshot,
        error: &AppError,
    ) -> Result<(), AppError> {
        self.dispatch(OverlayCommand::ShowError {
            selection: selection.clone(),
            error: error.clone(),
        })
    }

    async fn hide(&self) -> Result<(), AppError> {
        self.dispatch(OverlayCommand::Hide)
    }
}

/// Applies the non-activating policy to an NSPanel.
///
/// # Safety
///
/// `panel` must be a live NSPanel pointer and this function must execute on the
/// AppKit main thread. The function does not retain the panel.
pub unsafe fn configure_nonactivating_panel(
    panel: *mut c_void,
    policy: NonActivatingPanelPolicy,
) -> Result<(), &'static str> {
    if panel.is_null() {
        return Err("NSPanel pointer is null");
    }

    // SAFETY: caller guarantees a live NSPanel on the AppKit main thread; each
    // selector has the exact ABI represented by the typed helper.
    unsafe {
        send_usize(panel, "setStyleMask:", policy.style_mask)?;
        send_usize(panel, "setCollectionBehavior:", policy.collection_behavior)?;
        send_isize(panel, "setLevel:", policy.level)?;
        send_bool(panel, "setOpaque:", objc_bool(policy.opaque))?;
        send_bool(panel, "setHasShadow:", objc_bool(policy.has_shadow))?;
        send_bool(
            panel,
            "setHidesOnDeactivate:",
            objc_bool(policy.hides_on_deactivate),
        )?;
        send_bool(
            panel,
            "setReleasedWhenClosed:",
            objc_bool(policy.released_when_closed),
        )?;
        send_bool(
            panel,
            "setBecomesKeyOnlyIfNeeded:",
            objc_bool(policy.becomes_key_only_if_needed),
        )?;
    }
    Ok(())
}

/// Shows a configured panel without activating this application.
///
/// # Safety
///
/// `panel` must be a live NSPanel and the call must run on the AppKit main
/// thread.
pub unsafe fn order_front_without_activation(panel: *mut c_void) -> Result<(), &'static str> {
    if panel.is_null() {
        return Err("NSPanel pointer is null");
    }
    // SAFETY: selector is a no-argument NSWindow method and caller guarantees
    // thread and object validity.
    unsafe { send_no_args(panel, "orderFrontRegardless") }
}

/// Hides the reusable panel without destroying its content hierarchy.
///
/// # Safety
///
/// `panel` must be a live NSPanel and the call must run on the AppKit main
/// thread.
pub unsafe fn hide_panel(panel: *mut c_void) -> Result<(), &'static str> {
    if panel.is_null() {
        return Err("NSPanel pointer is null");
    }
    // SAFETY: orderOut: accepts a nullable sender and caller guarantees thread
    // and object validity.
    unsafe { send_object(panel, "orderOut:", std::ptr::null_mut()) }
}

fn objc_bool(value: bool) -> ObjcBool {
    if value {
        YES
    } else {
        NO
    }
}

fn selector(name: &str) -> Result<Sel, &'static str> {
    let name = CString::new(name).map_err(|_| "invalid Objective-C selector")?;
    // SAFETY: selector name is a valid NUL-terminated string.
    let selector = unsafe { sel_registerName(name.as_ptr()) };
    if selector.is_null() {
        Err("Objective-C selector could not be registered")
    } else {
        Ok(selector)
    }
}

unsafe fn send_usize(object: Id, name: &str, value: NSUInteger) -> Result<(), &'static str> {
    let selector = selector(name)?;
    type Send = unsafe extern "C" fn(Id, Sel, NSUInteger);
    // SAFETY: objc_msgSend is cast to the exact selector ABI.
    let send: Send = unsafe { mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(object, selector, value) };
    Ok(())
}

unsafe fn send_isize(object: Id, name: &str, value: NSInteger) -> Result<(), &'static str> {
    let selector = selector(name)?;
    type Send = unsafe extern "C" fn(Id, Sel, NSInteger);
    // SAFETY: objc_msgSend is cast to the exact selector ABI.
    let send: Send = unsafe { mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(object, selector, value) };
    Ok(())
}

unsafe fn send_bool(object: Id, name: &str, value: ObjcBool) -> Result<(), &'static str> {
    let selector = selector(name)?;
    type Send = unsafe extern "C" fn(Id, Sel, ObjcBool);
    // SAFETY: objc_msgSend is cast to the exact selector ABI.
    let send: Send = unsafe { mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(object, selector, value) };
    Ok(())
}

unsafe fn send_no_args(object: Id, name: &str) -> Result<(), &'static str> {
    let selector = selector(name)?;
    type Send = unsafe extern "C" fn(Id, Sel);
    // SAFETY: objc_msgSend is cast to the exact no-argument selector ABI.
    let send: Send = unsafe { mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(object, selector) };
    Ok(())
}

unsafe fn send_object(object: Id, name: &str, value: Id) -> Result<(), &'static str> {
    let selector = selector(name)?;
    type Send = unsafe extern "C" fn(Id, Sel, Id);
    // SAFETY: objc_msgSend is cast to the exact object-argument selector ABI.
    let send: Send = unsafe { mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(object, selector, value) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{
        contracts::{PhysicalRect, SelectionSnapshot},
        platform::OverlayController,
    };

    use super::{
        logical_window_bounds, place_overlay_in_screen_points, point_is_inside_overlay,
        MacOverlayWindow, MacScreenGeometry, NonActivatingPanelPolicy, OverlayCommand,
        CAN_JOIN_ALL_SPACES, FULL_SCREEN_AUXILIARY, IGNORES_CYCLE, NONACTIVATING_PANEL_MASK,
    };

    #[test]
    fn mixed_scale_secondary_display_uses_captured_quartz_screen_points() {
        // Captured from the user's Mac: the main display is Retina (2x), while
        // the secondary display is 1x and begins at Quartz x=1800.
        let screens = [
            MacScreenGeometry {
                id: "main".into(),
                logical_bounds: PhysicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1800.0,
                    height: 1169.0,
                },
            },
            MacScreenGeometry {
                id: "secondary".into(),
                logical_bounds: PhysicalRect {
                    x: 1800.0,
                    y: 0.0,
                    width: 2560.0,
                    height: 1440.0,
                },
            },
        ];
        let release_event = crate::platform::macos::PrimaryMouseEvent::Released {
            position: crate::placement::PhysicalPoint {
                x: 2200.0,
                y: 500.0,
            },
        };
        // The pointer may move during Accessibility's settle/retry delay. The
        // overlay must use the event's release coordinate, not sample this later.
        let pointer_after_resolution_delay = crate::placement::PhysicalPoint { x: 300.0, y: 300.0 };
        let crate::platform::macos::PrimaryMouseEvent::Released { position } = release_event else {
            unreachable!()
        };
        assert_ne!(position, pointer_after_resolution_delay);
        let release_point = PhysicalRect {
            x: position.x,
            y: position.y,
            width: 1.0,
            height: 1.0,
        };

        let placement = place_overlay_in_screen_points(
            release_point,
            &screens,
            crate::placement::PhysicalSize {
                width: 44.0,
                height: 44.0,
            },
            8.0,
        )
        .expect("secondary placement");

        assert_eq!(placement.screen_id, "secondary");
        assert_eq!(placement.size_logical_points.width, 44.0);
        assert!(placement.bounds().x >= 1800.0);
        let logical_bounds = logical_window_bounds(
            placement.position_logical_points,
            placement.size_logical_points,
            1.0,
        )
        .expect("secondary window bounds");
        let press_event = crate::platform::macos::PrimaryMouseEvent::Pressed {
            position: crate::placement::PhysicalPoint {
                x: logical_bounds.x + 22.0,
                y: logical_bounds.y + 22.0,
            },
        };
        let crate::platform::macos::PrimaryMouseEvent::Pressed { position } = press_event else {
            unreachable!()
        };
        assert!(point_is_inside_overlay(position, logical_bounds));
    }

    #[test]
    fn default_policy_preserves_foreground_application() {
        let policy = NonActivatingPanelPolicy::default();
        assert_ne!(policy.style_mask & NONACTIVATING_PANEL_MASK, 0);
        assert_ne!(policy.collection_behavior & CAN_JOIN_ALL_SPACES, 0);
        assert_ne!(policy.collection_behavior & FULL_SCREEN_AUXILIARY, 0);
        assert_ne!(policy.collection_behavior & IGNORES_CYCLE, 0);
        assert!(!policy.hides_on_deactivate);
        assert!(policy.becomes_key_only_if_needed);
    }

    #[tokio::test]
    async fn concrete_overlay_dispatches_correlated_state_and_hide() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&commands);
        let overlay = MacOverlayWindow::new(move |command| {
            recorded.lock().expect("command lock").push(command);
            Ok(())
        });
        let selection = SelectionSnapshot {
            id: 7,
            text: "selected".into(),
            source_application_id: Some("fixture.app".into()),
            bounds_physical_px: vec![PhysicalRect {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 10.0,
            }],
            anchor_physical_px: PhysicalRect {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 10.0,
            },
            captured_at_epoch_ms: 1,
        };

        overlay.show_button(&selection).await.expect("show");
        overlay.hide().await.expect("hide");

        let commands = commands.lock().expect("command lock");
        assert!(matches!(
            &commands[0],
            OverlayCommand::ShowButton { selection } if selection.id == 7
        ));
        assert_eq!(commands[1], OverlayCommand::Hide);
    }
}
