//! Production macOS adapters behind the convergence-owned module boundary.

#![cfg(target_os = "macos")]

pub mod input;
pub mod selection;
pub mod speech;
pub mod window;

pub use input::{PrimaryGestureState, PrimaryMouseEvent, PrimaryMouseObserver};
pub use selection::{
    normalize_rect, normalize_rects, AccessibilityPermission, DisplayTransform, MacSelectionAdapter,
};
pub use speech::MacSpeechAdapter;
pub use window::{
    configure_nonactivating_panel, hide_panel, order_front_without_activation, MacOverlayWindow,
    NonActivatingPanelPolicy, OverlayCommand,
};
