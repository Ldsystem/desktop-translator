use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use async_trait::async_trait;

use crate::{
    commands::{ApplicationCoordinator, MonitoringDecision},
    contracts::{
        AppError, AppErrorCode, PhysicalRect, SelectionSnapshot, TranslationRequest,
        TranslationResult,
    },
    coordinator::{CoordinatorEvent, OverlayState},
    overlay::OverlaySession,
    platform::{OverlayController, SelectionAdapter, SelectionPolicy, SpeechAdapter},
    services::{
        vocabulary::{VocabularyStore, VocabularyTranslationProvider},
        TranslationProvider,
    },
};

struct CountingProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl TranslationProvider for CountingProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(TranslationResult {
            selection_id: request.selection_id,
            translated_text: "hola".into(),
            detected_source_language: Some("en".into()),
            effective_source_language: "en".into(),
            target_language: request.target_language.clone(),
            part_of_speech: None,
            senses: Vec::new(),
        })
    }

    async fn supported_languages(&self) -> Result<Vec<String>, AppError> {
        Ok(vec!["en".into(), "es".into()])
    }
}

struct FixedSelectionAdapter {
    selection: SelectionSnapshot,
}

#[async_trait]
impl SelectionAdapter for FixedSelectionAdapter {
    async fn resolve_selection(&self, _: &SelectionPolicy) -> Result<SelectionSnapshot, AppError> {
        Ok(self.selection.clone())
    }
}

/// Stands in for a surface that publishes its accessibility tree lazily: it
/// reports no selection until it has been woken and asked enough times.
struct LazySelectionAdapter {
    selection: SelectionSnapshot,
    reads_before_selection_appears: usize,
    reads: AtomicUsize,
    wakes: AtomicUsize,
}

impl LazySelectionAdapter {
    fn new(reads_before_selection_appears: usize) -> Self {
        Self {
            selection: selection(),
            reads_before_selection_appears,
            reads: AtomicUsize::new(0),
            wakes: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SelectionAdapter for LazySelectionAdapter {
    async fn resolve_selection(&self, _: &SelectionPolicy) -> Result<SelectionSnapshot, AppError> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        if read < self.reads_before_selection_appears {
            return Err(AppError::new(
                AppErrorCode::NoSelection,
                "no selection",
                false,
            ));
        }
        Ok(self.selection.clone())
    }

    async fn prepare_source(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct RecordingOverlay {
    actions: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl OverlayController for RecordingOverlay {
    async fn show_button(&self, _: &SelectionSnapshot) -> Result<(), AppError> {
        self.actions.lock().expect("actions").push("button");
        Ok(())
    }

    async fn show_loading(&self, _: &SelectionSnapshot) -> Result<(), AppError> {
        self.actions.lock().expect("actions").push("loading");
        Ok(())
    }

    async fn show_result(
        &self,
        _: &SelectionSnapshot,
        _: &TranslationResult,
    ) -> Result<(), AppError> {
        self.actions.lock().expect("actions").push("result");
        Ok(())
    }

    async fn show_error(&self, _: &SelectionSnapshot, _: &AppError) -> Result<(), AppError> {
        self.actions.lock().expect("actions").push("error");
        Ok(())
    }

    async fn hide(&self) -> Result<(), AppError> {
        self.actions.lock().expect("actions").push("hide");
        Ok(())
    }
}

#[derive(Default)]
struct RecordingSpeech {
    stops: AtomicUsize,
}

#[async_trait]
impl SpeechAdapter for RecordingSpeech {
    async fn is_available(&self, language: &str) -> bool {
        language == "en"
    }

    async fn speak(&self, _: &str, _: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), AppError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn selection() -> SelectionSnapshot {
    let bounds = PhysicalRect {
        x: 100.0,
        y: 100.0,
        width: 80.0,
        height: 24.0,
    };
    SelectionSnapshot {
        id: 7,
        text: "hello".into(),
        example_sentence: None,
        source_application_id: Some("fixture.app".into()),
        bounds_physical_px: vec![bounds],
        anchor_physical_px: bounds,
        captured_at_epoch_ms: 1,
    }
}

fn request() -> TranslationRequest {
    TranslationRequest {
        selection_id: 7,
        text: "hello".into(),
        example_sentence: None,
        source_language: "auto".into(),
        target_language: "es".into(),
    }
}

fn application_coordinator(
    provider: Arc<CountingProvider>,
    overlay: Arc<RecordingOverlay>,
    speech: Arc<RecordingSpeech>,
) -> ApplicationCoordinator {
    ApplicationCoordinator::new(
        Arc::new(FixedSelectionAdapter {
            selection: selection(),
        }),
        overlay,
        provider,
        speech,
        SelectionPolicy {
            max_code_points: 5_000,
            excluded_application_id: Some("com.desktop-translator.app".into()),
        },
        true,
    )
}

fn coordinator_with_selection(
    selection: Arc<dyn SelectionAdapter>,
    overlay: Arc<RecordingOverlay>,
) -> ApplicationCoordinator {
    ApplicationCoordinator::new(
        selection,
        overlay,
        Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
        }),
        Arc::new(RecordingSpeech::default()),
        SelectionPolicy {
            max_code_points: 5_000,
            excluded_application_id: Some("com.desktop-translator.app".into()),
        },
        true,
    )
}

#[tokio::test]
async fn a_press_wakes_the_surface_before_the_selection_is_read() {
    let adapter = Arc::new(LazySelectionAdapter::new(0));
    let coordinator =
        coordinator_with_selection(adapter.clone(), Arc::new(RecordingOverlay::default()));

    coordinator.pointer_down().await.expect("pointer down");

    assert_eq!(adapter.wakes.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_surface_that_wakes_late_still_produces_a_button() {
    let adapter = Arc::new(LazySelectionAdapter::new(1));
    let overlay = Arc::new(RecordingOverlay::default());
    let coordinator = coordinator_with_selection(adapter.clone(), overlay.clone());

    coordinator.pointer_down().await.expect("pointer down");
    coordinator.pointer_up().await.expect("pointer up");

    assert_eq!(adapter.reads.load(Ordering::SeqCst), 2);
    assert!(
        overlay.actions.lock().expect("actions").contains(&"button"),
        "a late-waking surface must still show the button"
    );
}

#[tokio::test]
async fn a_surface_with_no_selection_is_not_retried_forever() {
    let adapter = Arc::new(LazySelectionAdapter::new(usize::MAX));
    let overlay = Arc::new(RecordingOverlay::default());
    let coordinator = coordinator_with_selection(adapter.clone(), overlay.clone());

    coordinator.pointer_up().await.expect("pointer up");

    assert_eq!(adapter.reads.load(Ordering::SeqCst), 2);
    assert!(!overlay.actions.lock().expect("actions").contains(&"button"));
}

#[tokio::test]
async fn coordinator_reports_native_speech_availability() {
    let coordinator = application_coordinator(
        Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
        }),
        Arc::new(RecordingOverlay::default()),
        Arc::new(RecordingSpeech::default()),
    );

    assert!(coordinator.speech_available("en".into()).await);
    assert!(!coordinator.speech_available("zz".into()).await);
}

#[tokio::test]
async fn typed_input_translates_without_a_native_selection() {
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let overlay = Arc::new(RecordingOverlay::default());
    let coordinator = application_coordinator(
        provider.clone(),
        overlay.clone(),
        Arc::new(RecordingSpeech::default()),
    );

    let result = coordinator
        .translate_input(TranslationRequest {
            selection_id: 0,
            text: "hello".into(),
            example_sentence: None,
            source_language: "auto".into(),
            target_language: "es".into(),
        })
        .await
        .expect("typed input must translate without an owning selection");

    assert_eq!(result.translated_text, "hola");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    // The contextual overlay belongs to selection flow and must stay untouched.
    assert!(overlay.actions.lock().expect("actions").is_empty());
    assert!(matches!(coordinator.snapshot(), OverlayState::Idle { .. }));
}

#[tokio::test]
async fn typed_input_rejects_an_empty_request() {
    let coordinator = application_coordinator(
        Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
        }),
        Arc::new(RecordingOverlay::default()),
        Arc::new(RecordingSpeech::default()),
    );

    let error = coordinator
        .translate_input(TranslationRequest {
            selection_id: 0,
            text: "   ".into(),
            example_sentence: None,
            source_language: "auto".into(),
            target_language: "es".into(),
        })
        .await
        .expect_err("blank input must be rejected before the provider is called");

    assert_eq!(error.code, AppErrorCode::Internal);
}

#[tokio::test]
async fn selection_and_typed_input_share_the_same_vocabulary_cache() {
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let database = tempfile::NamedTempFile::new().expect("database file");
    let store = Arc::new(VocabularyStore::open(database.path()).expect("vocabulary store"));
    let cached: Arc<dyn TranslationProvider> = Arc::new(VocabularyTranslationProvider::new(
        provider.clone(),
        store.clone(),
    ));
    let coordinator = ApplicationCoordinator::new(
        Arc::new(FixedSelectionAdapter {
            selection: selection(),
        }),
        Arc::new(RecordingOverlay::default()),
        cached,
        Arc::new(RecordingSpeech::default()),
        SelectionPolicy {
            max_code_points: 5_000,
            excluded_application_id: Some("com.desktop-translator.app".into()),
        },
        true,
    );

    coordinator.pointer_down().await.expect("pointer down");
    coordinator.pointer_up().await.expect("pointer up");
    coordinator
        .translate(request())
        .await
        .expect("selection miss");
    let typed = coordinator
        .translate_input(TranslationRequest {
            selection_id: 0,
            ..request()
        })
        .await
        .expect("typed hit");

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(typed.selection_id, 0);
    let entries = store.list(None, 1).expect("entries");
    assert_eq!(entries[0].lookup_count, 2);
    assert_eq!(entries[0].recall_score, 20.0);
}

#[tokio::test]
async fn selection_does_not_call_provider_until_explicit_translate() {
    let provider = CountingProvider {
        calls: AtomicUsize::new(0),
    };
    let overlay = RecordingOverlay::default();
    let selected = selection();
    let state = OverlayState::initial()
        .reduce(CoordinatorEvent::PointerUp { request_id: 1 })
        .reduce(CoordinatorEvent::SelectionResolved {
            request_id: 1,
            selection: selected.clone(),
        });

    overlay.show_button(&selected).await.expect("show button");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

    let request = TranslationRequest {
        selection_id: selected.id,
        text: selected.text.clone(),
        example_sentence: selected.example_sentence.clone(),
        source_language: "auto".into(),
        target_language: "es".into(),
    };
    let translating = state.reduce(CoordinatorEvent::Translate);
    overlay.show_loading(&selected).await.expect("show loading");
    let result = provider.translate(&request).await.expect("translate");
    let visible = translating.reduce(CoordinatorEvent::TranslationResolved(result.clone()));
    overlay
        .show_result(&selected, &result)
        .await
        .expect("show result");

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(visible, OverlayState::ResultVisible { .. }));
    assert_eq!(
        overlay.actions.lock().expect("actions").as_slice(),
        ["button", "loading", "result"]
    );
}

#[tokio::test]
async fn disable_invalidates_content_and_hides_overlay() {
    let overlay = RecordingOverlay::default();
    let state = OverlayState::ButtonVisible {
        selection: selection(),
        generation: 1,
    }
    .reduce(CoordinatorEvent::Disable);
    overlay.hide().await.expect("hide");

    assert_eq!(state, OverlayState::Disabled);
    assert_eq!(
        overlay.actions.lock().expect("actions").as_slice(),
        ["hide"]
    );
}

#[tokio::test]
async fn global_release_resolves_selection_without_translating() {
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let overlay = Arc::new(RecordingOverlay::default());
    let speech = Arc::new(RecordingSpeech::default());
    let coordinator = application_coordinator(provider.clone(), overlay.clone(), speech);

    coordinator.pointer_down().await.expect("pointer down");
    coordinator.pointer_up().await.expect("pointer up");

    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        overlay.actions.lock().expect("actions").as_slice(),
        ["hide", "button"]
    );
    assert!(matches!(
        coordinator.snapshot(),
        OverlayState::ButtonVisible { .. }
    ));
}

#[tokio::test]
async fn explicit_translate_rejects_renderer_text_not_owned_by_current_selection() {
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let overlay = Arc::new(RecordingOverlay::default());
    let speech = Arc::new(RecordingSpeech::default());
    let coordinator = application_coordinator(provider.clone(), overlay, speech);
    coordinator.pointer_down().await.expect("pointer down");
    coordinator.pointer_up().await.expect("pointer up");

    let mut tampered = request();
    tampered.text = "renderer supplied different text".into();
    let error = coordinator
        .translate(tampered)
        .await
        .expect_err("tampered request must fail");

    assert_eq!(error.code, AppErrorCode::NoSelection);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pointer_invalidation_prevents_a_translation_result_from_becoming_visible() {
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let overlay = Arc::new(RecordingOverlay::default());
    let speech = Arc::new(RecordingSpeech::default());
    let coordinator = Arc::new(application_coordinator(provider, overlay.clone(), speech));
    coordinator.pointer_down().await.expect("pointer down");
    coordinator.pointer_up().await.expect("pointer up");

    let translating = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move { coordinator.translate(request()).await })
    };
    tokio::task::yield_now().await;
    coordinator.pointer_down().await.expect("invalidate");
    translating
        .await
        .expect("translation task")
        .expect_err("stale");

    assert!(!overlay.actions.lock().expect("actions").contains(&"result"));
}

#[tokio::test]
async fn disable_hides_content_stops_speech_and_rejects_translation() {
    let provider = Arc::new(CountingProvider {
        calls: AtomicUsize::new(0),
    });
    let overlay = Arc::new(RecordingOverlay::default());
    let speech = Arc::new(RecordingSpeech::default());
    let coordinator = application_coordinator(provider.clone(), overlay.clone(), speech.clone());
    coordinator.pointer_down().await.expect("pointer down");
    coordinator.pointer_up().await.expect("pointer up");

    coordinator.set_enabled(false).await.expect("disable");
    let error = coordinator
        .translate(request())
        .await
        .expect_err("disabled translation must fail");

    assert_eq!(error.code, AppErrorCode::NoSelection);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(speech.stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        overlay.actions.lock().expect("actions").last(),
        Some(&"hide")
    );
    assert_eq!(coordinator.snapshot(), OverlayState::Disabled);
}

#[test]
fn first_selection_is_buffered_until_renderer_readiness() {
    let mut session = OverlaySession::default();
    let selected = selection();

    assert!(session.present(selected.clone()).is_none());
    assert_eq!(session.renderer_ready(), Some(selected));
    assert!(session.renderer_ready().is_none());
}

#[test]
fn hide_before_readiness_drops_buffered_private_selection() {
    let mut session = OverlaySession::default();
    assert!(session.present(selection()).is_none());

    let stale_idle = session.hide();

    assert!(session.renderer_ready().is_none());
    let mut replacement = selection();
    replacement.id += 1;
    assert!(session.present(replacement).is_some());
    assert!(!session.claim_idle_destruction(stale_idle));
}

#[test]
fn denied_startup_preserves_preference_but_tray_retries_enable() {
    let denied = MonitoringDecision::new(true, false);

    assert!(denied.preferred_enabled());
    assert!(!denied.effective_enabled());
    assert!(denied.tray_toggle_request());

    let granted = MonitoringDecision::new(denied.preferred_enabled(), true);
    assert!(granted.effective_enabled());
    assert_eq!(denied.desired_monitor_change(false), None);
    assert_eq!(denied.desired_monitor_change(true), Some(false));
    assert_eq!(granted.desired_monitor_change(false), Some(true));
    assert_eq!(granted.desired_monitor_change(true), None);
}

#[test]
fn idle_destruction_is_cancelled_by_new_content_and_resets_readiness() {
    let mut session = OverlaySession::default();
    assert_eq!(session.renderer_ready(), None);
    assert!(session.present(selection()).is_some());
    let stale_idle = session.hide();

    let mut replacement = selection();
    replacement.id += 1;
    assert!(session.present(replacement).is_some());
    assert!(!session.claim_idle_destruction(stale_idle));

    let current_idle = session.hide();
    assert!(session.claim_idle_destruction(current_idle));
    assert!(!session.is_renderer_ready());
}
