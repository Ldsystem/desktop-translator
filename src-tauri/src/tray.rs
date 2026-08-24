//! Tray/menu-bar indicator and lazy settings window.

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder,
};

use crate::{
    commands::RuntimeState,
    contracts::{AppError, AppErrorCode, UiLocale},
    services::SettingsStore,
};

const SETTINGS_LABEL: &str = "settings";
const STUDY_LABEL: &str = "study";
const ENABLED_ID: &str = "enabled";
const START_AT_LOGIN_ID: &str = "start-at-login";
const SETTINGS_ID: &str = "settings";
const STUDY_ID: &str = "study";
const QUIT_ID: &str = "quit";
const LOCALE_EN_ID: &str = "locale-en";
const LOCALE_ZH_ID: &str = "locale-zh-cn";

struct TrayCopy {
    selection: &'static str,
    login: &'static str,
    study: &'static str,
    settings: &'static str,
    quit: &'static str,
}

fn copy(locale: UiLocale) -> TrayCopy {
    match locale {
        UiLocale::English => TrayCopy {
            selection: "Selection Translation",
            login: "Start at Login",
            study: "Vocabulary Study…",
            settings: "Settings…",
            quit: "Quit",
        },
        UiLocale::SimplifiedChinese => TrayCopy {
            selection: "划词翻译",
            login: "登录时启动",
            study: "词汇学习…",
            settings: "设置…",
            quit: "退出",
        },
    }
}

/// Builds the application tray and its stable command menu.
pub fn install(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let settings = app
        .state::<RuntimeState>()
        .settings()
        .load()
        .map_err(|error| std::io::Error::other(error.message))?;
    let labels = copy(settings.ui_locale);
    let enabled = CheckMenuItem::with_id(
        app,
        ENABLED_ID,
        labels.selection,
        true,
        app.state::<RuntimeState>().monitoring_enabled(),
        None::<&str>,
    )?;
    let start_at_login = CheckMenuItem::with_id(
        app,
        START_AT_LOGIN_ID,
        labels.login,
        true,
        settings.start_at_login,
        None::<&str>,
    )?;
    let open_settings = MenuItem::with_id(app, SETTINGS_ID, labels.settings, true, None::<&str>)?;
    let open_study = MenuItem::with_id(app, STUDY_ID, labels.study, true, None::<&str>)?;
    let locale_en = CheckMenuItem::with_id(
        app,
        LOCALE_EN_ID,
        "English",
        true,
        settings.ui_locale == UiLocale::English,
        None::<&str>,
    )?;
    let locale_zh = CheckMenuItem::with_id(
        app,
        LOCALE_ZH_ID,
        "简体中文",
        true,
        settings.ui_locale == UiLocale::SimplifiedChinese,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_ID, labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &enabled,
            &start_at_login,
            &open_study,
            &open_settings,
            &locale_en,
            &locale_zh,
            &separator,
            &quit,
        ],
    )?;

    let enabled_item = enabled.clone();
    let login_item = start_at_login.clone();
    let settings_item = open_settings.clone();
    let study_item = open_study.clone();
    let quit_item = quit.clone();
    let locale_en_item = locale_en.clone();
    let locale_zh_item = locale_zh.clone();
    let changed_enabled = enabled.clone();
    let changed_login = start_at_login.clone();
    let changed_settings = open_settings.clone();
    let changed_study = open_study.clone();
    let changed_quit = quit.clone();
    let changed_locale_en = locale_en.clone();
    let changed_locale_zh = locale_zh.clone();
    app.listen("settings-changed", move |event| {
        let Ok(settings) = serde_json::from_str::<crate::contracts::UserSettings>(event.payload())
        else {
            return;
        };
        let labels = copy(settings.ui_locale);
        let _ = changed_enabled.set_text(labels.selection);
        let _ = changed_enabled.set_checked(settings.enabled);
        let _ = changed_login.set_text(labels.login);
        let _ = changed_login.set_checked(settings.start_at_login);
        let _ = changed_settings.set_text(labels.settings);
        let _ = changed_study.set_text(labels.study);
        let _ = changed_quit.set_text(labels.quit);
        let _ = changed_locale_en.set_checked(settings.ui_locale == UiLocale::English);
        let _ = changed_locale_zh.set_checked(settings.ui_locale == UiLocale::SimplifiedChinese);
    });
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
            STUDY_ID => {
                let _ = show_study(app);
            }
            LOCALE_EN_ID => {
                let _ = set_locale(app, UiLocale::English);
                let labels = copy(UiLocale::English);
                let _ = enabled_item.set_text(labels.selection);
                let _ = login_item.set_text(labels.login);
                let _ = settings_item.set_text(labels.settings);
                let _ = study_item.set_text(labels.study);
                let _ = quit_item.set_text(labels.quit);
                let _ = locale_en_item.set_checked(true);
                let _ = locale_zh_item.set_checked(false);
            }
            LOCALE_ZH_ID => {
                let _ = set_locale(app, UiLocale::SimplifiedChinese);
                let labels = copy(UiLocale::SimplifiedChinese);
                let _ = enabled_item.set_text(labels.selection);
                let _ = login_item.set_text(labels.login);
                let _ = settings_item.set_text(labels.settings);
                let _ = study_item.set_text(labels.study);
                let _ = quit_item.set_text(labels.quit);
                let _ = locale_en_item.set_checked(false);
                let _ = locale_zh_item.set_checked(true);
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

fn set_locale(app: &AppHandle, locale: UiLocale) -> Result<(), AppError> {
    let state = app.state::<RuntimeState>();
    let mut settings = state.settings().load()?;
    settings.ui_locale = locale;
    state.settings().save(&settings)?;
    app.emit("settings-changed", &settings)
        .map_err(|_| tray_error("Interface language could not be refreshed"))?;
    refresh_window_titles(app, locale);
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.emit("settings-changed", &settings);
        let _ = window.reload();
    }
    if let Some(window) = app.get_webview_window(STUDY_LABEL) {
        let _ = window.emit("settings-changed", &settings);
        let _ = window.reload();
    }
    Ok(())
}

/// Keeps native titles aligned when the locale changes from either settings or tray.
pub(crate) fn refresh_window_titles(app: &AppHandle, locale: UiLocale) {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.set_title(if locale == UiLocale::SimplifiedChinese {
            "桌面翻译设置"
        } else {
            "Desktop Translator Settings"
        });
    }
    if let Some(window) = app.get_webview_window(STUDY_LABEL) {
        let _ = window.set_title(if locale == UiLocale::SimplifiedChinese {
            "词汇学习"
        } else {
            "Vocabulary Study"
        });
    }
}

/// Shows or lazily creates the settings WebView.
pub fn show_settings(app: &AppHandle) -> Result<(), AppError> {
    let locale = app.state::<RuntimeState>().settings().load()?.ui_locale;
    let window = if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window
    } else {
        WebviewWindowBuilder::new(
            app,
            SETTINGS_LABEL,
            WebviewUrl::App("index.html?mode=settings".into()),
        )
        .title(if locale == UiLocale::SimplifiedChinese {
            "桌面翻译设置"
        } else {
            "Desktop Translator Settings"
        })
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

/// Shows or lazily creates the reusable vocabulary library and practice window.
pub fn show_study(app: &AppHandle) -> Result<(), AppError> {
    let locale = app.state::<RuntimeState>().settings().load()?.ui_locale;
    let window = if let Some(window) = app.get_webview_window(STUDY_LABEL) {
        window
    } else {
        WebviewWindowBuilder::new(
            app,
            STUDY_LABEL,
            WebviewUrl::App("index.html?mode=study".into()),
        )
        .title(if locale == UiLocale::SimplifiedChinese {
            "词汇学习"
        } else {
            "Vocabulary Study"
        })
        .inner_size(1040.0, 720.0)
        .min_inner_size(760.0, 560.0)
        .resizable(true)
        .skip_taskbar(false)
        .visible(false)
        .build()
        .map_err(|_| tray_error("Vocabulary study window could not be created"))?
    };
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|_| tray_error("Vocabulary study window could not be shown"))
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
