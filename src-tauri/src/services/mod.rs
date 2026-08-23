//! Provider, credential-vault, and non-secret settings service contracts.

pub mod credentials;
pub mod settings;
pub mod study;
pub mod textbooks;
pub mod translation;
pub mod vocabulary;

use async_trait::async_trait;

use crate::contracts::{
    AppError, LanguageCode, TranslationRequest, TranslationResult, UserSettings,
};

/// Translates validated requests without exposing provider details to the UI.
#[async_trait]
pub trait TranslationProvider: Send + Sync {
    /// Translates one request and preserves its selection correlation.
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError>;

    /// Returns provider-supported language identifiers.
    async fn supported_languages(&self) -> Result<Vec<LanguageCode>, AppError>;
}

/// Stores and validates the user API key inside the native process.
#[async_trait]
pub trait CredentialStore: Send + Sync {
    /// Replaces the credential-vault API key.
    fn set_api_key(&self, api_key: &str) -> Result<(), AppError>;

    /// Reads the key for internal service use only.
    fn get_api_key(&self) -> Result<Option<String>, AppError>;

    /// Validates the stored key without returning it to the renderer.
    async fn test_api_key(&self) -> Result<(), AppError>;

    /// Removes the stored credential.
    fn remove_api_key(&self) -> Result<(), AppError>;
}

/// Persists schema-versioned non-secret settings.
pub trait SettingsStore: Send + Sync {
    /// Loads validated settings or defaults.
    fn load(&self) -> Result<UserSettings, AppError>;

    /// Persists validated non-secret settings.
    fn save(&self, settings: &UserSettings) -> Result<(), AppError>;
}
