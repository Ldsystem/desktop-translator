//! CG-001 provider-independent data and error contracts shared with the WebView.

use serde::{Deserialize, Serialize};

/// BCP-47-style language identifier.
pub type LanguageCode = String;

/// Rectangle in global physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Immutable native selection correlated by a monotonic identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSnapshot {
    pub id: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_application_id: Option<String>,
    pub bounds_physical_px: Vec<PhysicalRect>,
    pub anchor_physical_px: PhysicalRect,
    pub captured_at_epoch_ms: u64,
}

/// User-selectable application color theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// Schema-versioned, non-secret application preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSettings {
    pub schema_version: u8,
    pub enabled: bool,
    pub source_language: LanguageCode,
    pub target_language: LanguageCode,
    pub start_at_login: bool,
    pub theme: Theme,
    pub max_selection_code_points: usize,
}

/// Validated request passed to a translation provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub selection_id: u64,
    pub text: String,
    pub source_language: LanguageCode,
    pub target_language: LanguageCode,
}

/// Provider-independent translation response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResult {
    pub selection_id: u64,
    pub translated_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_source_language: Option<LanguageCode>,
    pub effective_source_language: LanguageCode,
    pub target_language: LanguageCode,
}

/// One locally stored lexical item with independent demand and recall signals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyEntry {
    pub id: i64,
    pub source_text: String,
    pub translated_text: String,
    pub requested_source_language: LanguageCode,
    pub effective_source_language: LanguageCode,
    pub target_language: LanguageCode,
    pub lookup_count: u64,
    pub recall_score: f64,
    pub effective_recall: f64,
    pub familiarity_level: u8,
    pub review_count: u64,
    pub correct_count: u64,
    pub wrong_count: u64,
    pub correct_streak: u64,
    pub wrong_streak: u64,
    pub last_seen_epoch_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reviewed_epoch_ms: Option<u64>,
}

/// Monotonic invalidation signal emitted after native vocabulary state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VocabularyRevisionKind {
    Added,
    Updated,
    Deleted,
    LanguageCorrected,
    PracticeReviewed,
    Activated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyRevision {
    pub revision: u64,
    pub kind: VocabularyRevisionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<i64>,
}

/// Locally derived relationship between two vocabulary entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedVocabulary {
    pub entry: VocabularyEntry,
    pub reason: String,
}

/// Multiple-choice translation prompt selected entirely from local entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeQuestion {
    pub entry_id: i64,
    pub source_text: String,
    pub effective_source_language: LanguageCode,
    pub target_language: LanguageCode,
    pub choices: Vec<String>,
}

/// Result returned only after a practice answer is submitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeOutcome {
    pub correct: bool,
    pub correct_translation: String,
    pub entry: VocabularyEntry,
}

/// One pinned, app-curated textbook artifact offered for native installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextbookCatalogItem {
    pub id: String,
    pub title: String,
    pub source_language: LanguageCode,
    pub target_language: LanguageCode,
    pub version: String,
    pub download_url: String,
    pub expected_bytes: u64,
    pub sha256: String,
    pub license: String,
    pub attribution: String,
    pub source_url: String,
}

/// Installed textbook metadata safe to expose without local file paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledTextbook {
    pub id: String,
    pub title: String,
    pub source_language: LanguageCode,
    pub target_language: LanguageCode,
    pub version: String,
    pub license: String,
    pub attribution: String,
    pub source_url: String,
    pub entry_count: u64,
    pub installed_at_epoch_ms: u64,
    pub active: bool,
}

/// One normalized dictionary entry imported from a validated textbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextbookEntry {
    pub id: i64,
    pub textbook_id: String,
    pub source_text: String,
    pub translated_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phonetic_symbols: Option<String>,
    pub source_language: LanguageCode,
    pub target_language: LanguageCode,
}

/// Bounded page used for textbook browsing and search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextbookEntryPage {
    pub entries: Vec<TextbookEntry>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

/// Outcome of idempotently adding a textbook entry to the personal wordbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextbookPromotionResult {
    pub vocabulary_entry_id: i64,
    pub inserted: bool,
}

/// Durable attribution retained by a personal entry after its textbook is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyProvenance {
    pub textbook_id: String,
    pub textbook_title: String,
    pub textbook_version: String,
    pub license: String,
    pub attribution: String,
    pub source_url: String,
    pub source_text: String,
    pub translated_text: String,
    pub promoted_at_epoch_ms: u64,
}

/// A related lexical item with identities that cannot be confused across stores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedWord {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocabulary_entry_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textbook_entry_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textbook_id: Option<String>,
    pub source_text: String,
    pub translated_text: String,
    pub source_language: LanguageCode,
    pub target_language: LanguageCode,
    pub reason: String,
    pub promoted: bool,
    pub origins: Vec<RelatedOrigin>,
}

/// Display-safe origin metadata retained when identical related pairs are merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedOrigin {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textbook_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textbook_title: Option<String>,
}

/// User-selectable prompt direction for local practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PracticeDirection {
    Random,
    SourceToTarget,
    TargetToSource,
}

/// Persisted study preferences, deliberately separate from application settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticePreferences {
    pub direction: PracticeDirection,
}

/// Direction-neutral multiple-choice prompt selected from personal vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyPracticeQuestion {
    pub entry_id: i64,
    pub direction: PracticeDirection,
    pub prompt: String,
    pub prompt_language: LanguageCode,
    pub answer_language: LanguageCode,
    pub choices: Vec<String>,
}

/// Direction-aware result returned only after one explicit submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyPracticeOutcome {
    pub correct: bool,
    pub correct_answer: String,
    pub direction: PracticeDirection,
    pub entry: VocabularyEntry,
}

/// Stable error categories safe to expose across IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppErrorCode {
    PermissionDenied,
    UnsupportedControl,
    NoSelection,
    MissingCredential,
    InvalidCredential,
    ApiRestricted,
    BillingRequired,
    QuotaExceeded,
    Offline,
    Timeout,
    ServiceUnavailable,
    InvalidLanguagePair,
    Internal,
}

/// Stable error envelope without credentials or raw provider bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: AppErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl AppError {
    /// Creates a stable application error.
    pub fn new(code: AppErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

/// Semantic validation applied after deserialization at trust boundaries.
pub trait ValidateContract {
    /// Rejects values that are structurally valid but semantically unsafe.
    fn validate(&self) -> Result<(), AppError>;
}

const JS_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;

fn validation_error(message: impl Into<String>) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

impl ValidateContract for SelectionSnapshot {
    fn validate(&self) -> Result<(), AppError> {
        if self.id > JS_SAFE_INTEGER_MAX
            || self.captured_at_epoch_ms > JS_SAFE_INTEGER_MAX
            || self.text.trim().is_empty()
        {
            return Err(validation_error("selection violates schema constraints"));
        }
        if self.bounds_physical_px.is_empty()
            || self
                .bounds_physical_px
                .iter()
                .chain(std::iter::once(&self.anchor_physical_px))
                .any(|rect| {
                    !rect.x.is_finite()
                        || !rect.y.is_finite()
                        || !rect.width.is_finite()
                        || !rect.height.is_finite()
                        || rect.width <= 0.0
                        || rect.height <= 0.0
                })
        {
            return Err(validation_error(
                "selection bounds must be finite and positive",
            ));
        }
        Ok(())
    }
}

impl ValidateContract for UserSettings {
    fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1
            || self.source_language.trim().is_empty()
            || self.target_language.trim().is_empty()
            || self.max_selection_code_points == 0
            || u64::try_from(self.max_selection_code_points)
                .map(|value| value > JS_SAFE_INTEGER_MAX)
                .unwrap_or(true)
        {
            return Err(validation_error("settings violate schema constraints"));
        }
        Ok(())
    }
}

impl ValidateContract for TranslationRequest {
    fn validate(&self) -> Result<(), AppError> {
        if self.selection_id > JS_SAFE_INTEGER_MAX
            || self.text.trim().is_empty()
            || self.source_language.trim().is_empty()
            || self.target_language.trim().is_empty()
        {
            return Err(validation_error(
                "translation request contains an empty field",
            ));
        }
        Ok(())
    }
}

impl ValidateContract for TranslationResult {
    fn validate(&self) -> Result<(), AppError> {
        if self.selection_id > JS_SAFE_INTEGER_MAX
            || self.translated_text.trim().is_empty()
            || self.effective_source_language.trim().is_empty()
            || self.target_language.trim().is_empty()
            || self
                .detected_source_language
                .as_ref()
                .is_some_and(|language| language.trim().is_empty())
        {
            return Err(validation_error(
                "translation result contains an empty field",
            ));
        }
        Ok(())
    }
}

impl ValidateContract for TextbookCatalogItem {
    fn validate(&self) -> Result<(), AppError> {
        let required = [
            self.id.as_str(),
            self.title.as_str(),
            self.source_language.as_str(),
            self.target_language.as_str(),
            self.version.as_str(),
            self.license.as_str(),
            self.attribution.as_str(),
        ];
        if required.iter().any(|value| value.trim().is_empty())
            || !self.download_url.starts_with("https://")
            || !self.source_url.starts_with("https://")
            || self.expected_bytes == 0
            || self.expected_bytes > JS_SAFE_INTEGER_MAX
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(validation_error("textbook catalog item is unsafe"));
        }
        Ok(())
    }
}

impl ValidateContract for AppError {
    fn validate(&self) -> Result<(), AppError> {
        if self.message.trim().is_empty() {
            return Err(validation_error("error message must not be empty"));
        }
        Ok(())
    }
}

/// Decodes JSON and applies the contract's semantic validation.
pub fn decode_validated<T>(raw: &str) -> Result<T, AppError>
where
    T: for<'de> Deserialize<'de> + ValidateContract,
{
    let value: T = serde_json::from_str(raw)
        .map_err(|_| validation_error("contract JSON could not be decoded"))?;
    value.validate()?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{
        decode_validated, AppError, SelectionSnapshot, TranslationRequest, TranslationResult,
        UserSettings, VocabularyRevision,
    };

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixtures {
        selection: SelectionSnapshot,
        settings: UserSettings,
        translation_request: TranslationRequest,
        translation_result: TranslationResult,
        vocabulary_revision: VocabularyRevision,
        error: AppError,
        errors: Vec<AppError>,
    }

    #[test]
    fn shared_json_fixtures_deserialize_and_round_trip() {
        let raw = include_str!("../../src/contracts/fixtures.json");
        let fixtures: Fixtures = serde_json::from_str(raw).expect("fixtures must deserialize");

        assert_eq!(fixtures.selection.id, 42);
        assert_eq!(fixtures.settings.schema_version, 1);
        assert_eq!(fixtures.translation_request.selection_id, 42);
        assert_eq!(fixtures.translation_result.selection_id, 42);
        assert!(fixtures.error.retryable);
        assert_eq!(fixtures.vocabulary_revision.revision, 1);

        let original: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
        let round_trips = [
            serde_json::to_value(&fixtures.selection).expect("selection serializes"),
            serde_json::to_value(&fixtures.settings).expect("settings serialize"),
            serde_json::to_value(&fixtures.translation_request).expect("request serializes"),
            serde_json::to_value(&fixtures.translation_result).expect("result serializes"),
            serde_json::to_value(&fixtures.vocabulary_revision).expect("revision serializes"),
            serde_json::to_value(&fixtures.error).expect("error serializes"),
            serde_json::to_value(&fixtures.errors).expect("errors serialize"),
        ];
        assert_eq!(round_trips[0], original["selection"]);
        assert_eq!(round_trips[1], original["settings"]);
        assert_eq!(round_trips[2], original["translationRequest"]);
        assert_eq!(round_trips[3], original["translationResult"]);
        assert_eq!(round_trips[4], original["vocabularyRevision"]);
        assert_eq!(round_trips[5], original["error"]);
        assert_eq!(round_trips[6], original["errors"]);
    }

    #[test]
    fn validated_decode_rejects_semantically_invalid_contracts() {
        let invalid = r#"{
          "id": 1,
          "text": "",
          "boundsPhysicalPx": [{"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0}],
          "anchorPhysicalPx": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0},
          "capturedAtEpochMs": 1
        }"#;

        assert!(decode_validated::<SelectionSnapshot>(invalid).is_err());

        let invalid_result = r#"{
          "selectionId": 1,
          "translatedText": "hola",
          "detectedSourceLanguage": "",
          "effectiveSourceLanguage": "en",
          "targetLanguage": "es"
        }"#;
        assert!(decode_validated::<TranslationResult>(invalid_result).is_err());
    }
}
