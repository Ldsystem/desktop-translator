//! Schema-versioned persistence for non-secret application settings.
//!
//! The serialized type has no fields for selected text, translations, audio, credentials,
//! or source-application history, making content persistence unavailable by construction.

use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    contracts::{
        AppError, AppErrorCode, MicrosoftCloud, Theme, TranslationProviderId, UiLocale,
        UserSettings, ValidateContract,
    },
    services::SettingsStore,
};

/// Current on-disk settings schema.
pub const SETTINGS_SCHEMA_VERSION: u8 = 2;
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySettingsV0 {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    start_at_login: Option<bool>,
    #[serde(default)]
    theme: Option<Theme>,
    #[serde(default)]
    max_selection_code_points: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySettingsV1 {
    enabled: bool,
    source_language: String,
    target_language: String,
    start_at_login: bool,
    theme: Theme,
    max_selection_code_points: usize,
}

/// Returns safe first-run defaults containing no user content.
pub fn default_user_settings() -> UserSettings {
    UserSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        enabled: true,
        source_language: "auto".to_owned(),
        target_language: "en".to_owned(),
        start_at_login: false,
        theme: Theme::System,
        max_selection_code_points: 5_000,
        ui_locale: UiLocale::English,
        translation_provider: TranslationProviderId::Google,
        microsoft_cloud: MicrosoftCloud::Global,
        microsoft_region: None,
    }
}

/// JSON settings store using same-directory atomic replacement.
pub struct JsonSettingsStore {
    path: PathBuf,
    defaults: UserSettings,
}

impl JsonSettingsStore {
    pub fn new(path: impl Into<PathBuf>, defaults: UserSettings) -> Result<Self, AppError> {
        defaults.validate()?;
        Ok(Self {
            path: path.into(),
            defaults,
        })
    }

    pub fn with_application_defaults(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            defaults: default_user_settings(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Result<UserSettings, AppError> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(self.defaults.clone());
            }
            Err(_) => return Err(settings_io_error("Settings could not be read.")),
        };
        if metadata.len() > MAX_SETTINGS_BYTES {
            return self.replace_with_defaults();
        }

        let raw =
            fs::read(&self.path).map_err(|_| settings_io_error("Settings could not be read."))?;
        let value: serde_json::Value = match serde_json::from_slice(&raw) {
            Ok(value) => value,
            Err(_) => return self.replace_with_defaults(),
        };
        let schema = value.get("schemaVersion");

        match schema.and_then(serde_json::Value::as_u64) {
            Some(version) if version == u64::from(SETTINGS_SCHEMA_VERSION) => {
                let settings: UserSettings = match serde_json::from_value(value.clone()) {
                    Ok(settings) => settings,
                    Err(_) => return self.replace_with_defaults(),
                };
                if settings.validate().is_err() {
                    return self.replace_with_defaults();
                }
                let canonical =
                    serde_json::to_value(&settings).map_err(|_| settings_data_error())?;
                if canonical != value {
                    self.save(&settings)?;
                }
                Ok(settings)
            }
            Some(1) => {
                let legacy: LegacySettingsV1 = match serde_json::from_value(value) {
                    Ok(legacy) => legacy,
                    Err(_) => return self.replace_with_defaults(),
                };
                let migrated = UserSettings {
                    schema_version: SETTINGS_SCHEMA_VERSION,
                    enabled: legacy.enabled,
                    source_language: legacy.source_language,
                    target_language: legacy.target_language,
                    start_at_login: legacy.start_at_login,
                    theme: legacy.theme,
                    max_selection_code_points: legacy.max_selection_code_points,
                    ..self.defaults.clone()
                };
                migrated.validate().map_err(|_| settings_data_error())?;
                self.save(&migrated)?;
                Ok(migrated)
            }
            Some(0) | None => {
                let legacy: LegacySettingsV0 = match serde_json::from_value(value) {
                    Ok(legacy) => legacy,
                    Err(_) => return self.replace_with_defaults(),
                };
                let migrated = self.migrate_v0(legacy);
                if migrated.validate().is_err() {
                    return self.replace_with_defaults();
                }
                self.save(&migrated)?;
                Ok(migrated)
            }
            _ => self.replace_with_defaults(),
        }
    }

    fn save(&self, settings: &UserSettings) -> Result<(), AppError> {
        settings.validate().map_err(|_| settings_data_error())?;
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|_| settings_io_error("Settings directory could not be created."))?;

        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| settings_io_error("Settings temporary file could not be created."))?;
        {
            let mut writer = BufWriter::new(temporary.as_file_mut());
            serde_json::to_writer_pretty(&mut writer, settings)
                .map_err(|_| settings_data_error())?;
            writer
                .write_all(b"\n")
                .map_err(|_| settings_io_error("Settings could not be written."))?;
            writer
                .flush()
                .map_err(|_| settings_io_error("Settings could not be written."))?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| settings_io_error("Settings could not be synchronized."))?;
        temporary
            .persist(&self.path)
            .map_err(|_| settings_io_error("Settings could not be replaced atomically."))?;
        sync_parent_directory(parent)?;
        Ok(())
    }
}

impl JsonSettingsStore {
    fn migrate_v0(&self, legacy: LegacySettingsV0) -> UserSettings {
        let mut migrated = self.defaults.clone();
        migrated.schema_version = SETTINGS_SCHEMA_VERSION;
        if let Some(enabled) = legacy.enabled {
            migrated.enabled = enabled;
        }
        if let Some(source_language) = legacy.source_language {
            migrated.source_language = source_language;
        }
        if let Some(target_language) = legacy.target_language {
            migrated.target_language = target_language;
        }
        if let Some(start_at_login) = legacy.start_at_login {
            migrated.start_at_login = start_at_login;
        }
        if let Some(theme) = legacy.theme {
            migrated.theme = theme;
        }
        if let Some(max_selection_code_points) = legacy.max_selection_code_points {
            migrated.max_selection_code_points = max_selection_code_points;
        }
        migrated
    }

    fn replace_with_defaults(&self) -> Result<UserSettings, AppError> {
        self.defaults
            .validate()
            .map_err(|_| settings_data_error())?;
        self.save(&self.defaults)?;
        Ok(self.defaults.clone())
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), AppError> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| settings_io_error("Settings directory could not be synchronized."))
}

#[cfg(not(unix))]
fn sync_parent_directory(_: &Path) -> Result<(), AppError> {
    Ok(())
}

fn settings_data_error() -> AppError {
    AppError::new(
        AppErrorCode::Internal,
        "Settings are invalid or use an unsupported schema.",
        false,
    )
}

fn settings_io_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_loads_valid_schema_versioned_defaults() {
        let directory = tempfile::tempdir().expect("temp directory");
        let store = JsonSettingsStore::with_application_defaults(
            directory.path().join("nested/settings.json"),
        );
        let settings = store.load().expect("defaults load");
        assert_eq!(settings, default_user_settings());
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(!store.path().exists());
    }

    #[test]
    fn save_and_load_round_trip_only_non_secret_settings() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("state/settings.json");
        let store = JsonSettingsStore::with_application_defaults(&path);
        let mut settings = default_user_settings();
        settings.source_language = "de".to_owned();
        settings.target_language = "fr".to_owned();
        settings.theme = Theme::Dark;

        store.save(&settings).expect("settings save");
        assert_eq!(store.load().expect("settings load"), settings);

        let persisted = fs::read_to_string(path).expect("read persisted settings");
        let object = serde_json::from_str::<serde_json::Value>(&persisted)
            .expect("settings JSON")
            .as_object()
            .expect("settings object")
            .clone();
        let forbidden = [
            "text",
            "selectedText",
            "translatedText",
            "translation",
            "audio",
            "apiKey",
            "credential",
            "sourceApplication",
            "history",
        ];
        assert!(forbidden.iter().all(|field| !object.contains_key(*field)));
    }

    #[test]
    fn unsupported_schema_is_replaced_with_validated_defaults() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "schemaVersion": 2,
                "enabled": true,
                "sourceLanguage": "auto",
                "targetLanguage": "en",
                "startAtLogin": false,
                "theme": "system",
                "maxSelectionCodePoints": 5000
            }"#,
        )
        .expect("write fixture");
        let store = JsonSettingsStore::with_application_defaults(&path);
        assert_eq!(
            store.load().expect("defaults recover"),
            default_user_settings()
        );
        let recovered: UserSettings =
            serde_json::from_slice(&fs::read(path).expect("recovered settings"))
                .expect("valid recovered JSON");
        assert_eq!(recovered, default_user_settings());
    }

    #[test]
    fn schema_zero_migrates_deterministically_and_discards_unknown_content() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "schemaVersion": 0,
                "enabled": false,
                "sourceLanguage": "de",
                "targetLanguage": "fr",
                "startAtLogin": true,
                "selectedText": "must not survive",
                "apiKey": "synthetic-canary"
            }"#,
        )
        .expect("write legacy fixture");
        let store = JsonSettingsStore::with_application_defaults(&path);

        let migrated = store.load().expect("legacy settings migrate");
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(!migrated.enabled);
        assert_eq!(migrated.source_language, "de");
        assert_eq!(migrated.target_language, "fr");
        assert!(migrated.start_at_login);
        assert_eq!(migrated.theme, Theme::System);
        assert_eq!(migrated.max_selection_code_points, 5_000);

        let persisted = fs::read_to_string(path).expect("read migrated settings");
        assert!(!persisted.contains("must not survive"));
        assert!(!persisted.contains("synthetic-canary"));
        let persisted: UserSettings =
            serde_json::from_str(&persisted).expect("current schema persisted");
        assert_eq!(persisted, migrated);
    }

    #[test]
    fn schema_one_migrates_to_english_google_defaults_without_losing_preferences() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{
          "schemaVersion": 1,
          "enabled": false,
          "sourceLanguage": "en",
          "targetLanguage": "zh-CN",
          "startAtLogin": true,
          "theme": "dark",
          "maxSelectionCodePoints": 2048
        }"#,
        )
        .expect("legacy fixture");
        let settings = JsonSettingsStore::with_application_defaults(&path)
            .load()
            .expect("migration");
        assert_eq!(settings.schema_version, 2);
        assert_eq!(settings.ui_locale, UiLocale::English);
        assert_eq!(settings.translation_provider, TranslationProviderId::Google);
        assert_eq!(settings.microsoft_cloud, MicrosoftCloud::Global);
        assert_eq!(settings.max_selection_code_points, 2048);
        assert!(settings.start_at_login);
        assert_eq!(settings.theme, Theme::Dark);
    }

    #[test]
    fn malformed_data_is_replaced_without_retaining_raw_content() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{"selectedText":"must not survive","schemaVersion":"#,
        )
        .expect("write malformed fixture");
        let store = JsonSettingsStore::with_application_defaults(&path);

        assert_eq!(
            store.load().expect("defaults recover"),
            default_user_settings()
        );
        let recovered = fs::read_to_string(path).expect("read recovered settings");
        assert!(!recovered.contains("must not survive"));
        assert_eq!(
            serde_json::from_str::<UserSettings>(&recovered).expect("valid recovered settings"),
            default_user_settings()
        );
    }

    #[test]
    fn semantically_invalid_legacy_data_recovers_to_defaults() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "schemaVersion": 0,
                "sourceLanguage": "",
                "targetLanguage": ""
            }"#,
        )
        .expect("write invalid legacy fixture");
        let store = JsonSettingsStore::with_application_defaults(&path);

        assert_eq!(
            store.load().expect("defaults recover"),
            default_user_settings()
        );
    }

    #[test]
    fn invalid_settings_are_never_persisted() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        let store = JsonSettingsStore::with_application_defaults(&path);
        let mut settings = default_user_settings();
        settings.target_language.clear();
        assert!(store.save(&settings).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn replacement_leaves_one_complete_document_and_no_temporary_file() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("settings.json");
        let store = JsonSettingsStore::with_application_defaults(&path);
        let first = default_user_settings();
        store.save(&first).expect("first save");

        let mut second = first;
        second.target_language = "ja".to_owned();
        store.save(&second).expect("replacement save");

        assert_eq!(store.load().expect("replacement load"), second);
        let entries: Vec<_> = fs::read_dir(directory.path())
            .expect("read directory")
            .collect::<Result<_, _>>()
            .expect("directory entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path(), path);
    }

    #[test]
    fn custom_defaults_must_match_current_contract() {
        let mut invalid = default_user_settings();
        invalid.schema_version = 0;
        let error = JsonSettingsStore::new("settings.json", invalid)
            .err()
            .expect("invalid defaults rejected");
        assert_eq!(error.code, AppErrorCode::Internal);
    }
}
