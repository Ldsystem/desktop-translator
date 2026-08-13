//! Unelevated Windows adapters for selection, input, overlays, and speech.
//!
//! This module is compiled only by the convergence-owned
//! `cfg(target_os = "windows")` module declaration.

#![cfg(target_os = "windows")]

pub mod input;
pub mod selection;
pub mod speech;
pub mod window;

pub use input::{
    install_primary_mouse_hook, primary_mouse_event_channel, PrimaryMouseEvent,
    PrimaryMouseEventReceiver, PrimaryMouseEventSink, PrimaryMouseHook,
};
pub use selection::WindowsSelectionAdapter;
pub use speech::WindowsSpeechAdapter;
pub use window::{
    WindowsOverlayContent, WindowsOverlayContentDispatcher, WindowsOverlayMetrics,
    WindowsOverlayWindow,
};
