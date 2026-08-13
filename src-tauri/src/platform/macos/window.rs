//! AppKit policy for reusable, non-activating overlay panels.

use std::{
    ffi::{c_char, c_void, CString},
    mem,
    sync::Arc,
};

use async_trait::async_trait;

use crate::{
    contracts::{AppError, SelectionSnapshot, TranslationResult},
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
        MacOverlayWindow, NonActivatingPanelPolicy, OverlayCommand, CAN_JOIN_ALL_SPACES,
        FULL_SCREEN_AUXILIARY, IGNORES_CYCLE, NONACTIVATING_PANEL_MASK,
    };

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
