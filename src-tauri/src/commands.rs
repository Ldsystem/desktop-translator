//! Narrow validated Tauri commands and application service state.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use zeroize::Zeroize;

use crate::{
    contracts::{
        AppError, AppErrorCode, InstalledTextbook, PracticeDirection, PracticePreferences,
        RelatedWord, SelectionSnapshot, StudyPracticeOutcome, StudyPracticeQuestion,
        TextbookCatalogItem, TextbookEntryPage, TextbookPromotionResult, TranslationProviderId,
        TranslationRequest, TranslationResult, UserSettings, ValidateContract, VocabularyEntry,
        VocabularyProvenance, VocabularyRevision, VocabularyRevisionKind,
    },
    coordinator::{CoordinatorEvent, OverlayState},
    platform::{
        ObserverLifecycle, ObserverManager, OverlayController, SelectionAdapter, SelectionPolicy,
        SpeechAdapter,
    },
    services::{
        credentials::{ProviderCredentialStore, ProviderSecretField},
        settings::JsonSettingsStore,
        study::StudyService,
        textbooks::{curated_catalog, TextbookStore},
        translation::ProviderRouter,
        vocabulary::{
            is_vocabulary_eligible, TextbookTranslationProvider, VocabularyStore,
            VocabularyTranslationProvider,
        },
        SettingsStore, TranslationProvider,
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

    /// Whether monitoring should start or stop after a permission re-check.
    /// `None` means the running state already matches.
    pub(crate) fn desired_monitor_change(self, currently_enabled: bool) -> Option<bool> {
        let desired = self.effective_enabled();
        (desired != currently_enabled).then_some(desired)
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

    pub async fn supported_translation_languages(&self) -> Result<Vec<String>, AppError> {
        self.translation.supported_languages().await
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
    credentials: Arc<ProviderCredentialStore>,
    provider_router: Arc<ProviderRouter>,
    coordinator: Arc<ApplicationCoordinator>,
    overlay: Arc<crate::overlay::TauriOverlayController>,
    vocabulary: Arc<VocabularyStore>,
    textbooks: Arc<TextbookStore>,
    study: Arc<StudyService>,
    textbook_staging: PathBuf,
    observer: ObserverManager,
    vocabulary_revision: AtomicU64,
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
        let credentials = Arc::new(ProviderCredentialStore::new());
        let settings_service: Arc<dyn SettingsStore> = settings.clone();
        let provider_router = Arc::new(ProviderRouter::new(settings_service, credentials.clone())?);
        let upstream: Arc<dyn TranslationProvider> = provider_router.clone();
        let vocabulary_directory = app
            .path()
            .app_data_dir()
            .map_err(|_| internal_error("Application data path is unavailable"))?;
        std::fs::create_dir_all(&vocabulary_directory)
            .map_err(|_| internal_error("Application data directory could not be created"))?;
        let database_path = vocabulary_directory.join("vocabulary.sqlite3");
        let vocabulary = Arc::new(VocabularyStore::open(&database_path)?);
        let textbooks = Arc::new(TextbookStore::open(&database_path)?);
        textbooks.ensure_bundled_starter(now_epoch_ms())?;
        let study = Arc::new(StudyService::open(
            &database_path,
            vocabulary.clone(),
            textbooks.clone(),
        )?);
        let textbook_provider: Arc<dyn TranslationProvider> = Arc::new(
            TextbookTranslationProvider::new(upstream, textbooks.clone()),
        );
        let translation: Arc<dyn TranslationProvider> = Arc::new(
            VocabularyTranslationProvider::new(textbook_provider, vocabulary.clone()),
        );

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
            provider_router,
            coordinator,
            overlay,
            vocabulary,
            textbooks,
            study,
            textbook_staging: vocabulary_directory.join("textbook-downloads"),
            observer: ObserverManager::default(),
            vocabulary_revision: AtomicU64::new(0),
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

    #[cfg(target_os = "macos")]
    pub(crate) fn overlay_controller(&self) -> Arc<crate::overlay::TauriOverlayController> {
        self.overlay.clone()
    }

    /// Reports effective monitoring independently from persisted preference.
    pub fn monitoring_enabled(&self) -> bool {
        self.coordinator.is_enabled()
    }

    pub fn vocabulary(&self) -> &VocabularyStore {
        &self.vocabulary
    }

    pub fn emit_vocabulary_revision(
        &self,
        app: &AppHandle,
        kind: VocabularyRevisionKind,
        entry_id: Option<i64>,
    ) {
        let revision = self.vocabulary_revision.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = app.emit(
            "vocabulary-revision",
            VocabularyRevision {
                revision,
                kind,
                entry_id,
            },
        );
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
    let _ = app.emit("settings-changed", &settings);
    crate::tray::refresh_window_titles(&app, settings.ui_locale);
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
pub fn get_credential_status(
    state: State<'_, RuntimeState>,
    provider: Option<TranslationProviderId>,
) -> Result<&'static str, AppError> {
    let selected = provider.unwrap_or(state.settings.load()?.translation_provider);
    Ok(if state.credentials.configured(selected)? {
        "ready"
    } else {
        "missing"
    })
}

/// Opens a native secure prompt and stores the entered key directly in the OS vault.
#[tauri::command]
pub fn prompt_and_save_credential(
    state: State<'_, RuntimeState>,
    provider: TranslationProviderId,
    field: String,
) -> Result<bool, AppError> {
    let secret_field = match field.as_str() {
        "api-key" => ProviderSecretField::ApiKey,
        "app-id" if provider == TranslationProviderId::Baidu => ProviderSecretField::AppId,
        _ => {
            return Err(AppError::new(
                AppErrorCode::InvalidCredential,
                "Unsupported credential field.",
                false,
            ))
        }
    };
    let title = match (provider, secret_field) {
        (TranslationProviderId::Baidu, ProviderSecretField::AppId) => "Baidu Translation APP ID",
        (TranslationProviderId::Baidu, ProviderSecretField::ApiKey) => {
            "Baidu Translation Secret Key"
        }
        (TranslationProviderId::Microsoft, _) => "Microsoft Translator Subscription Key",
        _ => "Google Cloud Translation API Key",
    };
    let Some(mut api_key) = crate::credential_prompt::prompt_secure_text(
        title,
        "The key is stored directly in the operating-system credential vault.",
    )?
    else {
        return Ok(false);
    };
    let result = state.credentials.set(provider, secret_field, &api_key);
    api_key.zeroize();
    result.map(|_| true)
}

/// Validates the stored credential without returning credential material.
#[tauri::command]
pub async fn test_credential(
    state: State<'_, RuntimeState>,
    provider: Option<TranslationProviderId>,
) -> Result<(), AppError> {
    let selected = provider.unwrap_or(state.settings.load()?.translation_provider);
    state.provider_router.test(selected).await
}

/// Removes the stored credential.
#[tauri::command]
pub fn remove_credential(
    state: State<'_, RuntimeState>,
    provider: Option<TranslationProviderId>,
) -> Result<(), AppError> {
    let selected = provider.unwrap_or(state.settings.load()?.translation_provider);
    state.credentials.remove_provider(selected)
}

/// Translates only after the renderer's explicit Translate action.
#[tauri::command]
pub async fn translate_selection(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    request: TranslationRequest,
) -> Result<TranslationResult, AppError> {
    let tracks_vocabulary = is_vocabulary_eligible(&request.text);
    let result = state.coordinator.translate(request).await?;
    if tracks_vocabulary {
        state.emit_vocabulary_revision(&app, VocabularyRevisionKind::Updated, None);
    }
    Ok(result)
}

/// Translates text typed into the tray panel, which owns no native selection.
#[tauri::command]
pub async fn translate_input(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    request: TranslationRequest,
) -> Result<TranslationResult, AppError> {
    let tracks_vocabulary = is_vocabulary_eligible(&request.text);
    let result = state.coordinator.translate_input(request).await?;
    if tracks_vocabulary {
        state.emit_vocabulary_revision(&app, VocabularyRevisionKind::Updated, None);
    }
    Ok(result)
}

/// Searches or browses locally stored lexical items without exposing database access.
#[tauri::command]
pub fn list_vocabulary(
    state: State<'_, RuntimeState>,
    search: Option<String>,
) -> Result<Vec<VocabularyEntry>, AppError> {
    state.vocabulary.list_current(search.as_deref())
}

#[tauri::command]
pub fn list_vocabulary_provenance(
    state: State<'_, RuntimeState>,
    entry_id: i64,
) -> Result<Vec<VocabularyProvenance>, AppError> {
    state.vocabulary.provenance(entry_id)
}

#[tauri::command]
pub fn delete_vocabulary_entry(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    entry_id: i64,
) -> Result<(), AppError> {
    state.vocabulary.delete(entry_id)?;
    state.emit_vocabulary_revision(&app, VocabularyRevisionKind::Deleted, Some(entry_id));
    Ok(())
}

#[tauri::command]
pub async fn correct_vocabulary_source_language(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    entry_id: i64,
    effective_source_language: String,
) -> Result<VocabularyEntry, AppError> {
    let supported_languages = state.coordinator.supported_translation_languages().await?;
    let entry = state.vocabulary.correct_effective_source_language(
        entry_id,
        &effective_source_language,
        &supported_languages,
        now_epoch_ms(),
    )?;
    state.emit_vocabulary_revision(
        &app,
        VocabularyRevisionKind::LanguageCorrected,
        Some(entry_id),
    );
    Ok(entry)
}

/// Aggregates conservative relationships across personal and installed local corpora.
#[tauri::command]
pub fn get_related_vocabulary(
    state: State<'_, RuntimeState>,
    entry_id: i64,
    seed: Option<u64>,
) -> Result<Vec<RelatedWord>, AppError> {
    let now_ms = now_epoch_ms();
    state
        .study
        .related(entry_id, seed.unwrap_or(now_ms), now_ms)
}

#[tauri::command]
pub fn get_practice_preferences(
    state: State<'_, RuntimeState>,
) -> Result<PracticePreferences, AppError> {
    state.study.preferences()
}

#[tauri::command]
pub fn save_practice_preferences(
    state: State<'_, RuntimeState>,
    preferences: PracticePreferences,
) -> Result<(), AppError> {
    state.study.save_preferences(preferences)
}

/// Selects a personal practice candidate without recording a review.
#[tauri::command]
pub fn get_practice_question(
    state: State<'_, RuntimeState>,
) -> Result<Option<StudyPracticeQuestion>, AppError> {
    state.study.question(now_epoch_ms(), now_epoch_ms())
}

/// Scores one explicit answer and returns feedback after persistence succeeds.
#[tauri::command]
pub fn submit_practice_answer(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    entry_id: i64,
    direction: PracticeDirection,
    selected_answer: String,
) -> Result<StudyPracticeOutcome, AppError> {
    let outcome = state
        .study
        .submit(entry_id, direction, &selected_answer, now_epoch_ms())?;
    state.emit_vocabulary_revision(
        &app,
        VocabularyRevisionKind::PracticeReviewed,
        Some(entry_id),
    );
    Ok(outcome)
}

#[tauri::command]
pub fn list_textbook_catalog() -> Vec<TextbookCatalogItem> {
    curated_catalog()
}

#[tauri::command]
pub fn list_downloaded_textbooks(
    state: State<'_, RuntimeState>,
) -> Result<Vec<InstalledTextbook>, AppError> {
    state.textbooks.list_installed()
}

#[tauri::command]
pub async fn download_textbook(
    state: State<'_, RuntimeState>,
    textbook_id: String,
) -> Result<InstalledTextbook, AppError> {
    state
        .textbooks
        .download_and_install(&textbook_id, &state.textbook_staging, now_epoch_ms())
        .await
}

#[tauri::command]
pub fn set_active_textbook(
    state: State<'_, RuntimeState>,
    textbook_id: Option<String>,
) -> Result<(), AppError> {
    state.textbooks.set_active(textbook_id.as_deref())
}

#[tauri::command]
pub fn remove_downloaded_textbook(
    state: State<'_, RuntimeState>,
    textbook_id: String,
) -> Result<(), AppError> {
    state.textbooks.remove(&textbook_id)
}

#[tauri::command]
pub fn list_textbook_entries(
    state: State<'_, RuntimeState>,
    textbook_id: String,
    search: Option<String>,
    offset: u64,
    limit: u64,
) -> Result<TextbookEntryPage, AppError> {
    state
        .textbooks
        .list_entries(&textbook_id, search.as_deref(), offset, limit)
}

#[tauri::command]
pub fn add_textbook_entry_to_personal(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    textbook_entry_id: i64,
) -> Result<TextbookPromotionResult, AppError> {
    let result = state
        .textbooks
        .promote_entry(textbook_entry_id, now_epoch_ms())?;
    state.emit_vocabulary_revision(
        &app,
        VocabularyRevisionKind::Added,
        Some(result.vocabulary_entry_id),
    );
    Ok(result)
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

fn permission_status_label() -> &'static str {
    if platform_permission_granted() {
        "granted"
    } else {
        "denied"
    }
}

/// Reports platform permission without triggering a prompt.
#[tauri::command]
pub fn get_permission_status() -> &'static str {
    permission_status_label()
}

/// Re-reads Accessibility permission and starts or stops monitoring to match.
///
/// macOS does not always apply a newly granted toggle to a process that is
/// already running. Callers should still quit and relaunch after a first-time
/// grant; this path covers the cases where the kernel does update in place,
/// and recovers monitoring after Settings is reopened.
#[tauri::command]
pub async fn sync_permission(
    app: AppHandle,
    state: State<'_, RuntimeState>,
) -> Result<&'static str, AppError> {
    let granted = platform_permission_granted();
    let preferred = state.settings.load()?.enabled;
    let decision = MonitoringDecision::new(preferred, granted);
    match decision.desired_monitor_change(state.coordinator.is_enabled()) {
        Some(true) => {
            crate::start_global_monitor(&app)?;
            state.coordinator.set_enabled(true).await?;
        }
        Some(false) => {
            let disabled = state.coordinator.set_enabled(false).await;
            state.stop_monitor();
            disabled?;
        }
        None => {}
    }
    Ok(permission_status_label())
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

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
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
