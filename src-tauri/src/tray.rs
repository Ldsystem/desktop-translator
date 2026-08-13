//! Tray/menu-bar indicator and lazy settings window.

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};

use crate::{
    commands::RuntimeState,
    contracts::{AppError, AppErrorCode},
    services::SettingsStore,
};

const SETTINGS_LABEL: &str = "settings";
const ENABLED_ID: &str = "enabled";
const START_AT_LOGIN_ID: &str = "start-at-login";
const SETTINGS_ID: &str = "settings";
const QUIT_ID: &str = "quit";

/// Builds the application tray and its stable command menu.
pub fn install(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let settings = app
        .state::<RuntimeState>()
        .settings()
        .load()
        .map_err(|error| std::io::Error::other(error.message))?;
    let enabled = CheckMenuItem::with_id(
        app,
        ENABLED_ID,
        "Selection Translation",
        true,
        app.state::<RuntimeState>().monitoring_enabled(),
        None::<&str>,
    )?;
    let start_at_login = CheckMenuItem::with_id(
        app,
        START_AT_LOGIN_ID,
        "Start at Login",
        true,
        settings.start_at_login,
        None::<&str>,
    )?;
    let open_settings = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&enabled, &start_at_login, &open_settings, &separator, &quit],
    )?;

    let enabled_item = enabled.clone();
    let mut builder = TrayIconBuilder::new()
        .tooltip("Desktop Translator")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            ENABLED_ID => {
                let app = app.clone();
                let enabled_item = enabled_item.clone();
                tauri::async_runtime::spawn(async move {
                    if toggle_enabled(&app).await.is_err() {
                        let _ = show_settings(&app);
                    }
                    let checked = app.state::<RuntimeState>().monitoring_enabled();
                    let _ = enabled_item.set_checked(checked);
                });
            }
            START_AT_LOGIN_ID => {
                if let Err(error) = toggle_start_at_login(app) {
                    eprintln!("tray action failed: {}", error.message);
                }
            }
            SETTINGS_ID => {
                let _ = show_settings(app);
            }
            QUIT_ID => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<RuntimeState>();
                    let _ = state.coordinator().shutdown().await;
                    state.stop_monitor();
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                let _ = crate::quick_translate::toggle(tray.app_handle(), position);
            }
        });
    if let Some(icon) = tray_icon() {
        builder = builder.icon(icon);
        // A template image is tinted by macOS for light, dark, and highlighted
        // menu bars; the colored application icon is not.
        #[cfg(target_os = "macos")]
        {
            builder = builder.icon_as_template(true);
        }
    } else if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    app.manage(tray);
    Ok(())
}

/// Shows or lazily creates the settings WebView.
pub fn show_settings(app: &AppHandle) -> Result<(), AppError> {
    let window = if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window
    } else {
        WebviewWindowBuilder::new(
            app,
            SETTINGS_LABEL,
            WebviewUrl::App("index.html?mode=settings".into()),
        )
        .title("Desktop Translator Settings")
        .inner_size(680.0, 760.0)
        .min_inner_size(520.0, 560.0)
        .resizable(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|_| tray_error("Settings window could not be created"))?
    };
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|_| tray_error("Settings window could not be shown"))
}

async fn toggle_enabled(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<RuntimeState>();
    let enabled = !state.monitoring_enabled();
    crate::commands::set_enabled_from_native(app, enabled).await
}

fn toggle_start_at_login(app: &AppHandle) -> Result<(), AppError> {
    use tauri_plugin_autostart::ManagerExt;

    let state = app.state::<RuntimeState>();
    let mut settings = state.settings().load()?;
    let previous = settings.start_at_login;
    settings.start_at_login = !settings.start_at_login;
    let autolaunch = app.autolaunch();
    if settings.start_at_login {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    }
    .map_err(|_| tray_error("Start-at-login setting could not be updated"))?;
    if let Err(error) = state.settings().save(&settings) {
        if previous {
            let _ = autolaunch.enable();
        } else {
            let _ = autolaunch.disable();
        }
        return Err(error);
    }
    Ok(())
}

/// Decodes the monochrome menu-bar glyph, or `None` when it is unusable.
fn tray_icon() -> Option<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/tray-template.png")).ok()
}

fn tray_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}
