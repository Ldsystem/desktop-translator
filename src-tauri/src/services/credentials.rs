//! OS credential-vault storage for the Google Translation API key.
//!
//! Credential values never enter settings, logs, error messages, or renderer-facing results.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;

use crate::{
    contracts::{AppError, AppErrorCode},
    services::CredentialStore,
};

use super::translation::validate_google_api_key;

const DEFAULT_VAULT_SERVICE: &str = "com.desktop-translator.google-cloud-translation";
const DEFAULT_VAULT_ACCOUNT: &str = "translation-api-key";

/// Narrow abstraction around an operating-system credential vault.
pub trait CredentialVault: Send + Sync {
    fn set_secret(&self, secret: &str) -> Result<(), AppError>;
    fn get_secret(&self) -> Result<Option<String>, AppError>;
    fn remove_secret(&self) -> Result<(), AppError>;
}

/// `keyring`-backed macOS Keychain, Windows Credential Manager, or Linux secret-service vault.
pub struct KeyringVault {
    service: String,
    account: String,
}

impl KeyringVault {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Result<Self, AppError> {
        let service = service.into();
        let account = account.into();
        if service.trim().is_empty() || account.trim().is_empty() {
            return Err(vault_error());
        }
        Ok(Self { service, account })
    }

    pub fn application_default() -> Self {
        Self {
            service: DEFAULT_VAULT_SERVICE.to_owned(),
            account: DEFAULT_VAULT_ACCOUNT.to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, AppError> {
        keyring::Entry::new(&self.service, &self.account).map_err(|_| vault_error())
    }
}

impl CredentialVault for KeyringVault {
    fn set_secret(&self, secret: &str) -> Result<(), AppError> {
        self.entry()?
            .set_password(secret)
            .map_err(|_| vault_error())
    }

    fn get_secret(&self) -> Result<Option<String>, AppError> {
        match self.entry()?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(vault_error()),
        }
    }

    fn remove_secret(&self) -> Result<(), AppError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(vault_error()),
        }
    }
}

/// Validates a credential while keeping it inside the Rust process.
#[async_trait]
pub trait CredentialValidator: Send + Sync {
    async fn validate(&self, api_key: &str) -> Result<(), AppError>;
}

/// Google REST validator used by the production credential store.
pub struct GoogleCredentialValidator {
    client: Client,
}

impl GoogleCredentialValidator {
    pub fn new() -> Result<Self, AppError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .https_only(true)
            .build()
            .map_err(|_| vault_error())?;
        Ok(Self { client })
    }
}

#[async_trait]
impl CredentialValidator for GoogleCredentialValidator {
    async fn validate(&self, api_key: &str) -> Result<(), AppError> {
        validate_google_api_key(&self.client, api_key).await
    }
}

/// Credential service combining a vault backend and a provider validator.
pub struct VaultCredentialStore<V> {
    vault: V,
    validator: Arc<dyn CredentialValidator>,
}

impl<V> VaultCredentialStore<V>
where
    V: CredentialVault,
{
    pub fn new(vault: V, validator: Arc<dyn CredentialValidator>) -> Self {
        Self { vault, validator }
    }
}

impl VaultCredentialStore<KeyringVault> {
    /// Creates the application production credential service.
    pub fn application_default() -> Result<Self, AppError> {
        Ok(Self::new(
            KeyringVault::application_default(),
            Arc::new(GoogleCredentialValidator::new()?),
        ))
    }
}

#[async_trait]
impl<V> CredentialStore for VaultCredentialStore<V>
where
    V: CredentialVault,
{
    fn set_api_key(&self, api_key: &str) -> Result<(), AppError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidCredential,
                "Enter a Google Cloud Translation API key.",
                false,
            ));
        }
        self.vault.set_secret(api_key)
    }

    fn get_api_key(&self) -> Result<Option<String>, AppError> {
        self.vault.get_secret()
    }

    async fn test_api_key(&self) -> Result<(), AppError> {
        let api_key = self.vault.get_secret()?.ok_or_else(|| {
            AppError::new(
                AppErrorCode::MissingCredential,
                "Add a Google Cloud Translation API key in Settings.",
                false,
            )
        })?;
        self.validator.validate(&api_key).await
    }

    fn remove_api_key(&self) -> Result<(), AppError> {
        self.vault.remove_secret()
    }
}

fn vault_error() -> AppError {
    AppError::new(
        AppErrorCode::Internal,
        "The operating-system credential vault is unavailable.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemoryVault {
        secret: Mutex<Option<String>>,
        fail: bool,
    }

    impl CredentialVault for MemoryVault {
        fn set_secret(&self, secret: &str) -> Result<(), AppError> {
            if self.fail {
                return Err(vault_error());
            }
            *self.secret.lock().expect("vault lock") = Some(secret.to_owned());
            Ok(())
        }

        fn get_secret(&self) -> Result<Option<String>, AppError> {
            if self.fail {
                return Err(vault_error());
            }
            Ok(self.secret.lock().expect("vault lock").clone())
        }

        fn remove_secret(&self) -> Result<(), AppError> {
            if self.fail {
                return Err(vault_error());
            }
            *self.secret.lock().expect("vault lock") = None;
            Ok(())
        }
    }

    struct RecordingValidator {
        observed: Mutex<Vec<String>>,
        result: Result<(), AppError>,
    }

    #[async_trait]
    impl CredentialValidator for RecordingValidator {
        async fn validate(&self, api_key: &str) -> Result<(), AppError> {
            self.observed
                .lock()
                .expect("validator lock")
                .push(api_key.to_owned());
            self.result.clone()
        }
    }

    fn validator(result: Result<(), AppError>) -> Arc<RecordingValidator> {
        Arc::new(RecordingValidator {
            observed: Mutex::new(Vec::new()),
            result,
        })
    }

    #[test]
    fn set_trims_and_remove_is_idempotent_for_backend() {
        let validator = validator(Ok(()));
        let store = VaultCredentialStore::new(MemoryVault::default(), validator);
        store.set_api_key("  synthetic-key  ").expect("store key");
        assert_eq!(
            store.get_api_key().expect("read key").as_deref(),
            Some("synthetic-key")
        );
        store.remove_api_key().expect("remove key");
        store.remove_api_key().expect("repeat removal");
        assert_eq!(store.get_api_key().expect("read key"), None);
    }

    #[test]
    fn empty_keys_are_rejected_without_touching_vault() {
        let store = VaultCredentialStore::new(MemoryVault::default(), validator(Ok(())));
        let error = store.set_api_key(" \n ").unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidCredential);
        assert_eq!(store.get_api_key().expect("read key"), None);
    }

    #[tokio::test]
    async fn test_api_key_reads_internally_and_returns_only_validation_status() {
        let validator = validator(Ok(()));
        let store = VaultCredentialStore::new(MemoryVault::default(), validator.clone());
        store.set_api_key("synthetic-canary").expect("store key");

        assert_eq!(store.test_api_key().await, Ok(()));
        assert_eq!(
            validator
                .observed
                .lock()
                .expect("validator lock")
                .as_slice(),
            ["synthetic-canary"]
        );
    }

    #[tokio::test]
    async fn missing_key_does_not_invoke_validator() {
        let validator = validator(Ok(()));
        let store = VaultCredentialStore::new(MemoryVault::default(), validator.clone());
        let error = store.test_api_key().await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::MissingCredential);
        assert!(validator
            .observed
            .lock()
            .expect("validator lock")
            .is_empty());
    }

    #[tokio::test]
    async fn provider_error_is_forwarded_without_credential_content() {
        let provider_error = AppError::new(
            AppErrorCode::ApiRestricted,
            "The API key does not permit Cloud Translation Basic.",
            false,
        );
        let store = VaultCredentialStore::new(
            MemoryVault::default(),
            validator(Err(provider_error.clone())),
        );
        store.set_api_key("synthetic-canary").expect("store key");
        let error = store.test_api_key().await.unwrap_err();
        assert_eq!(error, provider_error);
        assert!(!error.message.contains("synthetic-canary"));
    }

    #[test]
    fn vault_failures_are_stable_and_redacted() {
        let store = VaultCredentialStore::new(
            MemoryVault {
                secret: Mutex::new(None),
                fail: true,
            },
            validator(Ok(())),
        );
        let error = store.set_api_key("synthetic-canary").unwrap_err();
        assert_eq!(error.code, AppErrorCode::Internal);
        assert!(!error.message.contains("synthetic-canary"));
    }
}
