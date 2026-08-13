//! Narrow validated Tauri commands and application service state.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tauri::{AppHandle, Manager, State, WebviewWindow};
use zeroize::Zeroize;

use crate::{
    contracts::{
        AppError, AppErrorCode, SelectionSnapshot, TranslationRequest, TranslationResult,
        UserSettings, ValidateContract,
    },
    coordinator::{CoordinatorEvent, OverlayState},
    platform::{
        ObserverLifecycle, ObserverManager, OverlayController, SelectionAdapter, SelectionPolicy,
        SpeechAdapter,
    },
    services::{
        credentials::{KeyringVault, VaultCredentialStore},
        settings::JsonSettingsStore,
        translation::GoogleTranslationProvider,
        CredentialStore, SettingsStore, TranslationProvider,
    },
};

#[cfg(target_os = "macos")]
use crate::platform::macos::{AccessibilityPermission, MacSelectionAdapter, MacSpeechAdapter};
#[cfg(target_os = "windows")]
use crate::platform::windows::{WindowsSelectionAdapter, WindowsSpeechAdapter};

const SELECTION_SETTLE_DELAY: Duration = Duration::from_millis(25);
/// A surface woken at pointer-down may still be publishing its accessibility
/// tree when the gesture ends, so one late retry follows an empty first read.
const SELECTION_RETRY_DELAY: Duration = Duration::from_millis(150);
const SELECTION_ATTEMPTS: usize = 2;
const APPLICATION_ID: &str = "com.desktoptranslator.desktop";

/// Separates persisted user intent from permission-gated runtime monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MonitoringDecision {
    preferred_enabled: bool,
    permission_granted: bool,
}

impl MonitoringDecision {
    pub(crate) fn new(preferred_enabled: bool, permission_granted: bool) -> Self {
        Self {
            preferred_enabled,
            permission_granted,
        }
    }

    #[cfg(test)]
    pub(crate) fn preferred_enabled(self) -> bool {
        self.preferred_enabled
    }

    pub(crate) fn effective_enabled(self) -> bool {
        self.preferred_enabled && self.permission_granted
    }

    #[cfg(test)]
    pub(crate) fn tray_toggle_request(self) -> bool {
        !self.effective_enabled()
    }
}

/// Owns the newest-selection-wins application flow independently of Tauri IPC.
pub struct ApplicationCoordinator {
    selection: Arc<dyn SelectionAdapter>,
    overlay: Arc<dyn OverlayController>,
    translation: Arc<dyn TranslationProvider>,
    speech: Arc<dyn SpeechAdapter>,
    policy: Mutex<SelectionPolicy>,
    state: Mutex<OverlayState>,
    next_request_id: AtomicU64,
    enabled: AtomicBool,
}

impl ApplicationCoordinator {
    /// Constructs one bounded coordinator suitable for production or fake adapters.
    pub fn new(
        selection: Arc<dyn SelectionAdapter>,
        overlay: Arc<dyn OverlayController>,
        translation: Arc<dyn TranslationProvider>,
        speech: Arc<dyn SpeechAdapter>,
        policy: SelectionPolicy,
        enabled: bool,
    ) -> Self {
        Self {
            selection,
            overlay,
            translation,
            speech,
            policy: Mutex::new(policy),
            state: Mutex::new(if enabled {
                OverlayState::initial()
            } else {
                OverlayState::Disabled
            }),
            // PointerDown advances the reducer generation from zero before the
            // first completed gesture, so request identifiers begin at two.
            next_request_id: AtomicU64::new(2),
            enabled: AtomicBool::new(enabled),
        }
    }

    /// Returns a content-bearing snapshot only for native tests and control flow.
    pub fn snapshot(&self) -> OverlayState {
        self.state.lock().expect("coordinator state").clone()
    }

    /// Reports the effective permission-gated monitoring state.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Invalidates visible and in-flight work as soon as a new press begins.
    pub async fn pointer_down(&self) -> Result<(), AppError> {
        if !self.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        self.reduce(CoordinatorEvent::PointerDown);
        self.selection.prepare_source().await;
        self.overlay.hide().await
    }

    /// Reads the selection, retrying once so a surface that was still waking at
    /// pointer-up is not reported as having no selection.
    async fn resolve_selection(
        &self,
        policy: &SelectionPolicy,
        request_id: u64,
    ) -> Option<SelectionSnapshot> {
        for attempt in 1..=SELECTION_ATTEMPTS {
            if !self.request_is_current(request_id) {
                return None;
            }
            match self.selection.resolve_selection(policy).await {
                Ok(selection) => return Some(selection),
                Err(_) if attempt < SELECTION_ATTEMPTS => {
                    tokio::time::sleep(SELECTION_RETRY_DELAY).await;
                }
                Err(_) => return None,
            }
        }
        None
    }

    /// Resolves the focused selection after a short settle delay without polling.
    pub async fn pointer_up(&self) -> Result<(), AppError> {
        if !self.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.reduce(CoordinatorEvent::PointerUp { request_id });
        tokio::time::sleep(SELECTION_SETTLE_DELAY).await;
        if !self.request_is_current(request_id) {
            return Ok(());
        }
        let policy = self.policy.lock().expect("selection policy").clone();
        match self.resolve_selection(&policy, request_id).await {
            Some(selection) => {
                let should_show = {
                    let mut state = self.state.lock().expect("coordinator state");
                    let next = state.clone().reduce(CoordinatorEvent::SelectionResolved {
                        request_id,
                        selection: selection.clone(),
                    });
                    let should_show = matches!(
                        &next,
                        OverlayState::ButtonVisible {
                            selection: current,
                            ..
                        } if current.id == selection.id
                    );
                    *state = next;
                    should_show
                };
                if should_show {
                    self.overlay.show_button(&selection).await?;
                }
            }
            None => {
                self.reduce(CoordinatorEvent::SelectionRejected { request_id });
            }
        }
        Ok(())
    }

    /// Translates only text owned by the current native selection.
    pub async fn translate(
        &self,
        request: TranslationRequest,
    ) -> Result<TranslationResult, AppError> {
        request.validate()?;
        let selection = {
            let mut state = self.state.lock().expect("coordinator state");
            let selection = visible_selection(&state)
                .filter(|selection| {
                    selection.id == request.selection_id && selection.text == request.text
                })
                .cloned()
                .ok_or_else(stale_request)?;
            let next = state.clone().reduce(CoordinatorEvent::Translate);
            if !matches!(next, OverlayState::Translating { .. }) {
                return Err(stale_request());
            }
            *state = next;
            selection
        };
        self.overlay.show_loading(&selection).await?;
        match self.translation.translate(&request).await {
            Ok(result) => {
                let current = {
                    let mut state = self.state.lock().expect("coordinator state");
                    let next = state
                        .clone()
                        .reduce(CoordinatorEvent::TranslationResolved(result.clone()));
                    let current = matches!(
                        &next,
                        OverlayState::ResultVisible {
                            selection: visible,
                            ..
                        } if visible.id == result.selection_id
                    );
                    *state = next;
                    current
                };
                if !current {
                    return Err(stale_request());
                }
                self.overlay.show_result(&selection, &result).await?;
                Ok(result)
            }
            Err(error) => {
                let current = {
                    let mut state = self.state.lock().expect("coordinator state");
                    let next = state.clone().reduce(CoordinatorEvent::TranslationFailed {
                        selection_id: request.selection_id,
                        error: error.clone(),
                    });
                    let current = matches!(
                        &next,
                        OverlayState::ErrorVisible {
                            selection: visible,
                            ..
                        } if visible.id == request.selection_id
                    );
                    *state = next;
                    current
                };
                if current {
                    self.overlay.show_error(&selection, &error).await?;
                    Err(error)
                } else {
                    Err(stale_request())
                }
            }
        }
    }

    /// Translates operator-typed text. The request owns no native selection, so
    /// the contextual overlay state is deliberately left untouched.
    pub async fn translate_input(
        &self,
        request: TranslationRequest,
    ) -> Result<TranslationResult, AppError> {
        request.validate()?;
        self.translation.translate(&request).await
    }

    /// Enables or disables monitoring state while retaining non-secret settings.
    pub async fn set_enabled(&self, enabled: bool) -> Result<(), AppError> {
        self.enabled.store(enabled, Ordering::Release);
        self.reduce(if enabled {
            CoordinatorEvent::Enable
        } else {
            CoordinatorEvent::Disable
        });
        if !enabled {
            let hide = self.overlay.hide().await;
            let stop = self.speech.stop().await;
            hide.and(stop)?;
        }
        Ok(())
    }

    /// Updates selection limits after settings persistence.
    pub fn update_policy(&self, policy: SelectionPolicy) {
        *self.policy.lock().expect("selection policy") = policy;
    }

    /// Dismisses and invalidates the contextual surface.
    pub async fn dismiss(&self) -> Result<(), AppError> {
        self.reduce(CoordinatorEvent::Dismiss);
        self.overlay.hide().await
    }

    /// Speaks only non-empty text locally.
    pub async fn speak(&self, text: String, language: String) -> Result<(), AppError> {
        if text.trim().is_empty() || language.trim().is_empty() {
            return Err(internal_error("Speech input is invalid"));
        }
        self.speech.speak(&text, &language).await
    }

    /// Reports whether an installed operating-system voice supports a language.
    pub async fn speech_available(&self, language: String) -> bool {
        !language.trim().is_empty() && self.speech.is_available(&language).await
    }

    /// Invalidates work and stops native side effects before application exit.
    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.set_enabled(false).await
    }

    fn reduce(&self, event: CoordinatorEvent) {
        let mut state = self.state.lock().expect("coordinator state");
        *state = state.clone().reduce(event);
    }

    fn request_is_current(&self, request_id: u64) -> bool {
        matches!(
            self.snapshot(),
            OverlayState::ResolvingSelection {
                request_id: current,
                ..
            } if current == request_id
        )
    }
}

/// Long-lived native services shared by validated commands.
pub struct RuntimeState {
    settings: Arc<JsonSettingsStore>,
    credentials: Arc<VaultCredentialStore<KeyringVault>>,
    coordinator: Arc<ApplicationCoordinator>,
    overlay: Arc<crate::overlay::TauriOverlayController>,
    observer: ObserverManager,
}

impl RuntimeState {
    /// Creates native services without exposing credential values to the renderer.
    pub fn initialize(app: &AppHandle) -> Result<Self, AppError> {
        let settings_path = app
            .path()
            .app_config_dir()
            .map_err(|_| internal_error("Application settings path is unavailable"))?
            .join("settings.json");
        let settings = Arc::new(JsonSettingsStore::with_application_defaults(settings_path));
        let credentials = Arc::new(VaultCredentialStore::application_default()?);
        let credential_provider: Arc<dyn CredentialStore> = credentials.clone();
        let translation = Arc::new(GoogleTranslationProvider::new(credential_provider)?);

        #[cfg(target_os = "macos")]
        let speech: Arc<dyn SpeechAdapter> = Arc::new(MacSpeechAdapter::new()?);
        #[cfg(target_os = "windows")]
        let speech: Arc<dyn SpeechAdapter> = Arc::new(WindowsSpeechAdapter::new()?);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        compile_error!("Desktop Translator supports only macOS and Windows");

        #[cfg(target_os = "macos")]
        let selection: Arc<dyn SelectionAdapter> =
            Arc::new(MacSelectionAdapter::with_live_displays());
        #[cfg(target_os = "windows")]
        let selection: Arc<dyn SelectionAdapter> = Arc::new(WindowsSelectionAdapter::new()?);
        let loaded_settings = settings.load()?;
        let enabled =
            MonitoringDecision::new(loaded_settings.enabled, platform_permission_granted())
                .effective_enabled();
        let overlay = Arc::new(crate::overlay::TauriOverlayController::new(app.clone()));
        let coordinator = Arc::new(ApplicationCoordinator::new(
            selection,
            overlay.clone(),
            translation,
            speech,
            selection_policy(&loaded_settings),
            enabled,
        ));

        Ok(Self {
            settings,
            credentials,
            coordinator,
            overlay,
            observer: ObserverManager::default(),
        })
    }

    /// Exposes the non-secret settings service to native tray integration.
    pub fn settings(&self) -> &JsonSettingsStore {
        &self.settings
    }

    /// Exposes the coordinator to the global event worker.
    pub fn coordinator(&self) -> Arc<ApplicationCoordinator> {
        self.coordinator.clone()
    }

    /// Reports effective monitoring independently from persisted preference.
    pub fn monitoring_enabled(&self) -> bool {
        self.coordinator.is_enabled()
    }

    /// Flushes a buffered first-use selection after renderer subscription.
    pub fn overlay_renderer_ready(&self) -> Result<(), AppError> {
        self.overlay.renderer_ready()
    }

    /// Claims the single process-wide input observer startup.
    pub fn claim_monitor_start(&self) -> bool {
        self.observer.begin_start()
    }

    /// Releases a failed observer startup so permission rechecks can retry.
    pub fn release_monitor_start(&self) {
        self.observer.cancel_start();
    }

    /// Installs the wakeable owner for the active process-wide observer.
    pub fn install_monitor(&self, monitor: ObserverLifecycle) -> bool {
        self.observer.finish_start(monitor)
    }

    /// Wakes and joins the active observer before returning.
    pub fn stop_monitor(&self) {
        self.observer.stop();
    }
}

/// Returns schema-validated non-secret settings.
#[tauri::command]
pub fn get_settings(state: State<'_, RuntimeState>) -> Result<UserSettings, AppError> {
    state.settings.load()
}

/// Persists schema-validated non-secret settings.
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    settings: UserSettings,
) -> Result<(), AppError> {
    settings.validate()?;
    if settings.enabled && !platform_permission_granted() {
        return Err(AppError::new(
            AppErrorCode::PermissionDenied,
            "Accessibility permission is required before monitoring can be enabled.",
            false,
        ));
    }
    let previous = state.settings.load()?;
    sync_start_at_login(&app, settings.start_at_login)?;
    if let Err(error) = state.settings.save(&settings) {
        let _ = sync_start_at_login(&app, previous.start_at_login);
        return Err(error);
    }
    state.coordinator.update_policy(selection_policy(&settings));
    if settings.enabled {
        if let Err(error) = crate::start_global_monitor(&app) {
            let _ = state.settings.save(&previous);
            let _ = sync_start_at_login(&app, previous.start_at_login);
            state.coordinator.update_policy(selection_policy(&previous));
            return Err(error);
        }
        state.coordinator.set_enabled(true).await?;
    } else {
        let disabled = state.coordinator.set_enabled(false).await;
        state.stop_monitor();
        disabled?;
    }
    Ok(())
}

/// Reports only whether a key exists; the key never crosses IPC.
#[tauri::command]
pub fn get_credential_status(state: State<'_, RuntimeState>) -> Result<&'static str, AppError> {
    Ok(if state.credentials.get_api_key()?.is_some() {
        "ready"
    } else {
        "missing"
    })
}

/// Opens a native secure prompt and stores the entered key directly in the OS vault.
#[tauri::command]
pub fn prompt_and_save_credential(state: State<'_, RuntimeState>) -> Result<bool, AppError> {
    let Some(mut api_key) = crate::credential_prompt::prompt_secure_text(
        "Google Cloud Translation API Key",
        "The key is stored directly in the operating-system credential vault.",
    )?
    else {
        return Ok(false);
    };
    let result = state.credentials.set_api_key(&api_key);
    api_key.zeroize();
    result.map(|_| true)
}

/// Validates the stored credential without returning credential material.
#[tauri::command]
pub async fn test_credential(state: State<'_, RuntimeState>) -> Result<(), AppError> {
    state.credentials.test_api_key().await
}

/// Removes the stored credential.
#[tauri::command]
pub fn remove_credential(state: State<'_, RuntimeState>) -> Result<(), AppError> {
    state.credentials.remove_api_key()
}

/// Translates only after the renderer's explicit Translate action.
#[tauri::command]
pub async fn translate_selection(
    state: State<'_, RuntimeState>,
    request: TranslationRequest,
) -> Result<TranslationResult, AppError> {
    state.coordinator.translate(request).await
}

/// Translates text typed into the tray panel, which owns no native selection.
#[tauri::command]
pub async fn translate_input(
    state: State<'_, RuntimeState>,
    request: TranslationRequest,
) -> Result<TranslationResult, AppError> {
    state.coordinator.translate_input(request).await
}

/// Speaks source or translated text using the local operating system.
#[tauri::command]
pub async fn speak_text(
    state: State<'_, RuntimeState>,
    text: String,
    language: String,
) -> Result<(), AppError> {
    state.coordinator.speak(text, language).await
}

/// Reports native voice availability without exposing platform-specific details.
#[tauri::command]
pub async fn get_speech_availability(
    state: State<'_, RuntimeState>,
    language: String,
) -> Result<bool, AppError> {
    Ok(state.coordinator.speech_available(language).await)
}

/// Stops active native speech.
#[tauri::command]
pub async fn stop_speech(state: State<'_, RuntimeState>) -> Result<(), AppError> {
    state.coordinator.speech.stop().await
}

/// Hides the reusable contextual surface.
#[tauri::command]
pub async fn dismiss_overlay(state: State<'_, RuntimeState>) -> Result<(), AppError> {
    state.coordinator.dismiss().await
}

/// Completes the event-listener readiness handshake for the contextual renderer.
#[tauri::command]
pub fn overlay_ready(
    window: WebviewWindow,
    state: State<'_, RuntimeState>,
) -> Result<(), AppError> {
    if window.label() != "overlay" {
        return Err(internal_error("Overlay readiness has an invalid origin"));
    }
    state.overlay_renderer_ready()
}

/// Opens the platform accessibility-permission settings page.
#[tauri::command]
pub fn open_accessibility_settings() -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", "ms-settings:easeofaccess"]);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|_| internal_error("System accessibility settings could not be opened"))
}

/// Reports platform permission without triggering a prompt.
#[tauri::command]
pub fn get_permission_status() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        match MacSelectionAdapter::permission_status() {
            crate::platform::macos::AccessibilityPermission::Granted => "granted",
            crate::platform::macos::AccessibilityPermission::Denied => "denied",
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::any::TypeId::of::<WindowsSelectionAdapter>();
        "granted"
    }
}

/// Quits through Tauri so managed windows and state are dropped.
#[tauri::command]
pub async fn quit_application(
    app: AppHandle,
    state: State<'_, RuntimeState>,
) -> Result<(), AppError> {
    let _ = state.coordinator.shutdown().await;
    state.stop_monitor();
    app.exit(0);
    Ok(())
}

/// Applies native tray enablement through the same permission and invalidation path.
pub(crate) async fn set_enabled_from_native(
    app: &AppHandle,
    enabled: bool,
) -> Result<(), AppError> {
    if enabled && !platform_permission_granted() {
        return Err(AppError::new(
            AppErrorCode::PermissionDenied,
            "Accessibility permission is required before monitoring can be enabled.",
            false,
        ));
    }
    let state = app.state::<RuntimeState>();
    let mut settings = state.settings.load()?;
    let previous_enabled = settings.enabled;
    settings.enabled = enabled;
    state.settings.save(&settings)?;
    if enabled {
        if let Err(error) = crate::start_global_monitor(app) {
            settings.enabled = previous_enabled;
            let _ = state.settings.save(&settings);
            return Err(error);
        }
        state.coordinator.set_enabled(true).await?;
    } else {
        let disabled = state.coordinator.set_enabled(false).await;
        state.stop_monitor();
        disabled?;
    }
    Ok(())
}

fn sync_start_at_login(app: &AppHandle, enabled: bool) -> Result<(), AppError> {
    use tauri_plugin_autostart::ManagerExt;

    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    }
    .map_err(|_| internal_error("Start-at-login setting could not be updated"))
}

fn internal_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

fn visible_selection(state: &OverlayState) -> Option<&crate::contracts::SelectionSnapshot> {
    match state {
        OverlayState::ButtonVisible { selection, .. }
        | OverlayState::ResultVisible { selection, .. }
        | OverlayState::ErrorVisible { selection, .. } => Some(selection),
        _ => None,
    }
}

fn stale_request() -> AppError {
    AppError::new(
        AppErrorCode::NoSelection,
        "The selection is no longer current.",
        false,
    )
}

fn selection_policy(settings: &UserSettings) -> SelectionPolicy {
    SelectionPolicy {
        max_code_points: settings.max_selection_code_points,
        excluded_application_id: Some(APPLICATION_ID.into()),
    }
}

fn platform_permission_granted() -> bool {
    #[cfg(target_os = "macos")]
    {
        MacSelectionAdapter::permission_status() == AccessibilityPermission::Granted
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
}
