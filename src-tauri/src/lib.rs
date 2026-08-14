//! Native core for the Desktop Translator Tauri application.

use tauri::Manager;

use crate::{
    commands::RuntimeState,
    contracts::{AppError, AppErrorCode},
    coordinator::OverlayState,
    platform::ObserverLifecycle,
};

pub mod commands;
pub mod contracts;
pub mod coordinator;
pub mod credential_prompt;
pub mod overlay;
pub mod placement;
pub mod platform;
pub mod quick_translate;
pub mod services;
pub mod tray;

#[cfg(test)]
mod integration_tests;

fn builder() -> tauri::Builder<tauri::Wry> {
    use commands::{
        dismiss_overlay, get_credential_status, get_permission_status, get_settings,
        get_speech_availability, open_accessibility_settings, overlay_ready,
        prompt_and_save_credential, quit_application, remove_credential, save_settings, speak_text,
        stop_speech, sync_permission, test_credential, translate_input, translate_selection,
        RuntimeState,
    };
    use tauri_plugin_autostart::MacosLauncher;

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_credential_status,
            prompt_and_save_credential,
            test_credential,
            remove_credential,
            translate_selection,
            translate_input,
            get_speech_availability,
            speak_text,
            stop_speech,
            dismiss_overlay,
            overlay_ready,
            open_accessibility_settings,
            get_permission_status,
            sync_permission,
            quit_application
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let state = RuntimeState::initialize(app.handle())
                .map_err(|error| std::io::Error::other(error.message))?;
            app.manage(state);
            if !matches!(
                app.state::<RuntimeState>().coordinator().snapshot(),
                OverlayState::Disabled
            ) {
                start_global_monitor(app.handle())
                    .map_err(|error| std::io::Error::other(error.message))?;
            }
            tray::install(app)?;
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // The tray panel behaves like a popover: losing focus dismisses it.
            tauri::WindowEvent::Focused(false)
                if window.label() == crate::quick_translate::QUICK_LABEL =>
            {
                let _ = window.hide();
            }
            tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "overlay" => {
                let coordinator = window.state::<RuntimeState>().coordinator();
                tauri::async_runtime::spawn(async move {
                    let _ = coordinator.dismiss().await;
                });
            }
            _ => {}
        })
}

/// Starts one blocking, event-driven primary-button worker for the host platform.
pub(crate) fn start_global_monitor(app: &tauri::AppHandle) -> Result<(), AppError> {
    let state = app.state::<RuntimeState>();
    if !state.claim_monitor_start() {
        return Ok(());
    }
    let coordinator = state.coordinator();

    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos::{PrimaryMouseEvent, PrimaryMouseObserver};

        let app_handle = app.clone();
        let observer = PrimaryMouseObserver::start().map_err(|_| {
            state.release_monitor_start();
            AppError::new(
                AppErrorCode::PermissionDenied,
                "Accessibility permission is required before monitoring can be enabled.",
                false,
            )
        })?;
        let stop = observer.stop_handle();
        let worker = std::thread::Builder::new()
            .name("global-primary-input".into())
            .spawn(move || {
                let mut routing = PrimaryGestureRouting::default();
                while let Ok(event) = observer.recv() {
                    let should_forward = match event {
                        PrimaryMouseEvent::Pressed => routing.should_forward_press(
                            crate::overlay::cursor_is_over_overlay(&app_handle),
                        ),
                        PrimaryMouseEvent::Released => routing.should_forward_release(),
                    };
                    if !should_forward {
                        continue;
                    }
                    let result = tauri::async_runtime::block_on(async {
                        match event {
                            PrimaryMouseEvent::Pressed => coordinator.pointer_down().await,
                            PrimaryMouseEvent::Released => coordinator.pointer_up().await,
                        }
                    });
                    if result.is_err() {
                        let _ = tauri::async_runtime::block_on(coordinator.dismiss());
                    }
                }
            })
            .map_err(|_| {
                state.release_monitor_start();
                monitor_error()
            })?;
        if !state.install_monitor(ObserverLifecycle::new(move || stop.stop(), worker)) {
            return Err(monitor_error());
        }
    }

    #[cfg(target_os = "windows")]
    {
        use crate::platform::windows::{
            install_primary_mouse_hook, primary_mouse_event_channel, PrimaryMouseEvent,
        };

        let app_handle = app.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("global-primary-input".into())
            .spawn(move || {
                let (sink, receiver) = primary_mouse_event_channel();
                let hook = match install_primary_mouse_hook(sink) {
                    Ok(hook) => {
                        let _ = ready_tx.send(Ok(receiver.stop_handle()));
                        hook
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let mut routing = PrimaryGestureRouting::default();
                while let Ok(event) = receiver.recv() {
                    let should_forward = match event {
                        PrimaryMouseEvent::Pressed => routing.should_forward_press(
                            crate::overlay::cursor_is_over_overlay(&app_handle),
                        ),
                        PrimaryMouseEvent::Released => routing.should_forward_release(),
                    };
                    if !should_forward {
                        continue;
                    }
                    let result = tauri::async_runtime::block_on(async {
                        match event {
                            PrimaryMouseEvent::Pressed => coordinator.pointer_down().await,
                            PrimaryMouseEvent::Released => coordinator.pointer_up().await,
                        }
                    });
                    if result.is_err() {
                        let _ = tauri::async_runtime::block_on(coordinator.dismiss());
                    }
                }
                drop(hook);
            })
            .map_err(|_| {
                state.release_monitor_start();
                monitor_error()
            })?;
        let stop = match ready_rx.recv() {
            Ok(Ok(stop)) => stop,
            Ok(Err(error)) => {
                let _ = worker.join();
                state.release_monitor_start();
                return Err(error);
            }
            Err(_) => {
                let _ = worker.join();
                state.release_monitor_start();
                return Err(monitor_error());
            }
        };
        if !state.install_monitor(ObserverLifecycle::new(move || stop.stop(), worker)) {
            return Err(monitor_error());
        }
    }

    Ok(())
}

fn monitor_error() -> AppError {
    AppError::new(
        AppErrorCode::Internal,
        "The global input observer could not be started.",
        false,
    )
}

#[derive(Default)]
struct PrimaryGestureRouting {
    began_inside_overlay: bool,
}

impl PrimaryGestureRouting {
    fn should_forward_press(&mut self, inside_overlay: bool) -> bool {
        self.began_inside_overlay = inside_overlay;
        !inside_overlay
    }

    fn should_forward_release(&mut self) -> bool {
        !std::mem::take(&mut self.began_inside_overlay)
    }
}

/// Starts the Tauri application and blocks until shutdown.
pub fn run() {
    let application = builder()
        .build(tauri::generate_context!())
        .expect("failed to build desktop translator");
    application.run(|app, event| match event {
        tauri::RunEvent::Resumed => {
            let coordinator = app.state::<RuntimeState>().coordinator();
            tauri::async_runtime::spawn(async move {
                let _ = coordinator.dismiss().await;
            });
        }
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            if should_prevent_implicit_exit(code) {
                api.prevent_exit();
            } else {
                shutdown_runtime(app);
            }
        }
        tauri::RunEvent::Exit => {
            shutdown_runtime(app);
        }
        _ => {}
    });
}

fn should_prevent_implicit_exit(code: Option<i32>) -> bool {
    code.is_none()
}

fn shutdown_runtime(app: &tauri::AppHandle) {
    let state = app.state::<RuntimeState>();
    let coordinator = state.coordinator();
    let _ = tauri::async_runtime::block_on(coordinator.shutdown());
    state.stop_monitor();
}

#[cfg(test)]
mod tests {
    #[test]
    fn constructs_application_builder() {
        let _builder = super::builder();
    }

    #[test]
    fn tray_only_lifecycle_prevents_implicit_exit_but_allows_explicit_quit() {
        assert!(super::should_prevent_implicit_exit(None));
        assert!(!super::should_prevent_implicit_exit(Some(0)));
    }

    #[test]
    fn overlay_originated_pointer_gesture_is_not_forwarded_to_selection_monitoring() {
        let mut routing = super::PrimaryGestureRouting::default();

        assert!(!routing.should_forward_press(true));
        assert!(!routing.should_forward_release());
        assert!(routing.should_forward_press(false));
        assert!(routing.should_forward_release());
    }
}
