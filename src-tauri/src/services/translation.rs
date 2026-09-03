//! Google Cloud Translation Basic REST adapter.
//!
//! This module intentionally emits no request, response, credential, or text logs.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    contracts::{
        AppError, AppErrorCode, LanguageCode, MicrosoftCloud, PartOfSpeech, TranslationProviderId,
        TranslationRequest, TranslationResult, TranslationSense, ValidateContract,
    },
    services::{
        credentials::{ProviderCredentialStore, ProviderSecretField},
        CredentialStore, SettingsStore, TranslationProvider,
    },
};

const TRANSLATE_ENDPOINT: &str = "https://translation.googleapis.com/language/translate/v2";
const LANGUAGES_ENDPOINT: &str =
    "https://translation.googleapis.com/language/translate/v2/languages";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);
const DEFAULT_MAX_ATTEMPTS: usize = 2;
const MAX_PROVIDER_BODY_BYTES: usize = 64 * 1024;

/// Production Google Translation Basic provider.
pub struct GoogleTranslationProvider {
    client: Client,
    credentials: Arc<dyn CredentialStore>,
    translate_endpoint: String,
    languages_endpoint: String,
    max_attempts: usize,
}

impl GoogleTranslationProvider {
    /// Builds the outbound client. Proxy discovery is left at reqwest's default
    /// so the operating system's proxy configuration is honoured; many networks
    /// reach Google only through one.
    fn build_client() -> Result<Client, AppError> {
        Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .https_only(true)
            .build()
            .map_err(|_| internal_error("translation client could not be initialized"))
    }

    /// Creates a provider with a bounded request timeout and one retry.
    pub fn new(credentials: Arc<dyn CredentialStore>) -> Result<Self, AppError> {
        let client = Self::build_client()?;

        Ok(Self {
            client,
            credentials,
            translate_endpoint: TRANSLATE_ENDPOINT.to_owned(),
            languages_endpoint: LANGUAGES_ENDPOINT.to_owned(),
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        })
    }

    #[cfg(test)]
    fn with_test_endpoints(
        client: Client,
        credentials: Arc<dyn CredentialStore>,
        translate_endpoint: impl Into<String>,
        languages_endpoint: impl Into<String>,
        max_attempts: usize,
    ) -> Self {
        Self {
            client,
            credentials,
            translate_endpoint: translate_endpoint.into(),
            languages_endpoint: languages_endpoint.into(),
            max_attempts: max_attempts.max(1),
        }
    }

    fn api_key(&self) -> Result<String, AppError> {
        self.credentials
            .get_api_key()?
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                AppError::new(
                    AppErrorCode::MissingCredential,
                    "Add a Google Cloud Translation API key in Settings.",
                    false,
                )
            })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleTranslateRequest<'a> {
    q: &'a str,
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    format: &'static str,
}

#[derive(Deserialize)]
struct GoogleTranslateEnvelope {
    data: GoogleTranslations,
}

#[derive(Deserialize)]
struct GoogleTranslations {
    translations: Vec<GoogleTranslation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleTranslation {
    translated_text: String,
    detected_source_language: Option<String>,
}

#[derive(Deserialize)]
struct GoogleLanguagesEnvelope {
    data: GoogleLanguages,
}

#[derive(Deserialize)]
struct GoogleLanguages {
    languages: Vec<GoogleLanguage>,
}

#[derive(Deserialize)]
struct GoogleLanguage {
    language: String,
}

#[derive(Default, Deserialize)]
struct GoogleErrorEnvelope {
    #[serde(default)]
    error: GoogleErrorBody,
}

#[derive(Default, Deserialize)]
struct GoogleErrorBody {
    #[serde(default)]
    code: u16,
    #[serde(default)]
    message: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    details: Vec<GoogleErrorDetail>,
}

#[derive(Default, Deserialize)]
struct GoogleErrorDetail {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    domain: String,
}

#[async_trait]
impl TranslationProvider for GoogleTranslationProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        request.validate()?;
        let api_key = self.api_key()?;
        let source = (!request.source_language.eq_ignore_ascii_case("auto"))
            .then_some(request.source_language.as_str());
        let payload = GoogleTranslateRequest {
            q: &request.text,
            target: &request.target_language,
            source,
            format: "text",
        };

        let body = send_json_with_retry(
            &self.client,
            reqwest::Method::POST,
            &self.translate_endpoint,
            &api_key,
            Some(&payload),
            self.max_attempts,
        )
        .await?;
        parse_translation_response(request, &body)
    }

    async fn supported_languages(&self) -> Result<Vec<LanguageCode>, AppError> {
        let api_key = self.api_key()?;
        let body = send_json_with_retry::<GoogleTranslateRequest<'_>>(
            &self.client,
            reqwest::Method::GET,
            &self.languages_endpoint,
            &api_key,
            None,
            self.max_attempts,
        )
        .await?;
        parse_languages_response(&body)
    }
}

/// Validates a key without returning provider details or credential material.
pub(crate) async fn validate_google_api_key(
    client: &Client,
    api_key: &str,
) -> Result<(), AppError> {
    if api_key.trim().is_empty() {
        return Err(AppError::new(
            AppErrorCode::MissingCredential,
            "Enter a Google Cloud Translation API key.",
            false,
        ));
    }

    send_json_with_retry::<GoogleTranslateRequest<'_>>(
        client,
        reqwest::Method::GET,
        LANGUAGES_ENDPOINT,
        api_key,
        None,
        DEFAULT_MAX_ATTEMPTS,
    )
    .await
    .map(|_| ())
}

async fn send_json_with_retry<T: Serialize + ?Sized>(
    client: &Client,
    method: reqwest::Method,
    endpoint: &str,
    api_key: &str,
    payload: Option<&T>,
    max_attempts: usize,
) -> Result<Vec<u8>, AppError> {
    let attempts = max_attempts.max(1);
    for attempt in 0..attempts {
        let mut request = client
            .request(method.clone(), endpoint)
            .header("x-goog-api-key", api_key)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(payload) = payload {
            request = request.json(payload);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let body = read_bounded_body(response).await?;
                if status.is_success() {
                    return Ok(body);
                }

                let error = map_http_error(status, &body);
                if !error.retryable || attempt + 1 == attempts {
                    return Err(error);
                }
            }
            Err(error) => {
                let mapped = map_transport_error(&error);
                if !mapped.retryable || attempt + 1 == attempts {
                    return Err(mapped);
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(150 * (attempt as u64 + 1))).await;
    }

    Err(internal_error("translation request failed"))
}

async fn read_bounded_body(response: reqwest::Response) -> Result<Vec<u8>, AppError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_BODY_BYTES as u64)
    {
        return Err(internal_error(
            "translation service returned an invalid response",
        ));
    }

    let body = response
        .bytes()
        .await
        .map_err(|error| map_transport_error(&error))?;
    if body.len() > MAX_PROVIDER_BODY_BYTES {
        return Err(internal_error(
            "translation service returned an invalid response",
        ));
    }
    Ok(body.to_vec())
}

fn parse_translation_response(
    request: &TranslationRequest,
    body: &[u8],
) -> Result<TranslationResult, AppError> {
    let envelope: GoogleTranslateEnvelope = serde_json::from_slice(body)
        .map_err(|_| internal_error("translation service returned an invalid response"))?;
    let translation = envelope
        .data
        .translations
        .into_iter()
        .next()
        .ok_or_else(|| internal_error("translation service returned no translation"))?;
    let translated_text =
        html_escape::decode_html_entities(&translation.translated_text).into_owned();
    let detected = translation
        .detected_source_language
        .filter(|language| !language.trim().is_empty());
    let effective_source_language = if request.source_language.eq_ignore_ascii_case("auto") {
        detected.clone().unwrap_or_else(|| "auto".to_owned())
    } else {
        request.source_language.clone()
    };

    let result = TranslationResult {
        selection_id: request.selection_id,
        translated_text: translated_text.clone(),
        detected_source_language: detected,
        effective_source_language,
        target_language: request.target_language.clone(),
        part_of_speech: None,
        senses: vec![TranslationSense {
            text: translated_text.clone(),
            part_of_speech: None,
            rank: 0,
            is_primary: true,
            confidence: None,
        }],
    };
    result.validate()?;
    Ok(result)
}

fn parse_languages_response(body: &[u8]) -> Result<Vec<LanguageCode>, AppError> {
    let envelope: GoogleLanguagesEnvelope = serde_json::from_slice(body)
        .map_err(|_| internal_error("translation service returned an invalid response"))?;
    let mut languages: Vec<_> = envelope
        .data
        .languages
        .into_iter()
        .map(|entry| entry.language)
        .filter(|language| !language.trim().is_empty())
        .collect();
    languages.sort_unstable();
    languages.dedup();
    if languages.is_empty() {
        return Err(internal_error(
            "translation service returned no supported languages",
        ));
    }
    Ok(languages)
}

fn map_transport_error(error: &reqwest::Error) -> AppError {
    if error.is_connect() {
        AppError::new(
            AppErrorCode::Offline,
            "Google Translation could not be reached. Check your connection.",
            true,
        )
    } else if error.is_timeout() {
        AppError::new(
            AppErrorCode::Timeout,
            "Google Translation timed out. Try again.",
            true,
        )
    } else {
        AppError::new(
            AppErrorCode::ServiceUnavailable,
            "Google Translation is temporarily unavailable. Try again.",
            true,
        )
    }
}

fn map_http_error(status: StatusCode, body: &[u8]) -> AppError {
    let provider = serde_json::from_slice::<GoogleErrorEnvelope>(body).unwrap_or_default();
    let mut classification = format!(
        "{} {} {}",
        provider.error.code, provider.error.status, provider.error.message
    )
    .to_ascii_lowercase();
    for detail in provider.error.details {
        classification.push(' ');
        classification.push_str(&detail.reason.to_ascii_lowercase());
        classification.push(' ');
        classification.push_str(&detail.domain.to_ascii_lowercase());
    }

    if status == StatusCode::TOO_MANY_REQUESTS
        || contains_any(
            &classification,
            &[
                "quota",
                "rate_limit",
                "rate limit",
                "resource_exhausted",
                "daily limit exceeded",
                "user rate limit exceeded",
            ],
        )
    {
        AppError::new(
            AppErrorCode::QuotaExceeded,
            "Google Translation quota is exhausted. Check quotas and try later.",
            false,
        )
    } else if contains_any(
        &classification,
        &["billing", "billing_not_active", "billing_disabled"],
    ) {
        AppError::new(
            AppErrorCode::BillingRequired,
            "Enable billing for the Google Cloud project.",
            false,
        )
    } else if status == StatusCode::UNAUTHORIZED
        || contains_any(
            &classification,
            &[
                "api_key_invalid",
                "keyinvalid",
                "invalid api key",
                "api key not valid",
            ],
        )
    {
        AppError::new(
            AppErrorCode::InvalidCredential,
            "The Google Cloud Translation API key is invalid.",
            false,
        )
    } else if status == StatusCode::FORBIDDEN
        || contains_any(
            &classification,
            &["api_key_service_blocked", "permission_denied", "forbidden"],
        )
    {
        AppError::new(
            AppErrorCode::ApiRestricted,
            "The API key does not permit Cloud Translation Basic.",
            false,
        )
    } else if status == StatusCode::BAD_REQUEST {
        AppError::new(
            AppErrorCode::InvalidLanguagePair,
            "Google Translation rejected the language pair.",
            false,
        )
    } else if status.is_server_error() {
        AppError::new(
            AppErrorCode::ServiceUnavailable,
            "Google Translation is temporarily unavailable. Try again.",
            true,
        )
    } else {
        internal_error("Google Translation rejected the request")
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn internal_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

const BAIDU_ENDPOINT: &str = "https://fanyi-api.baidu.com/api/trans/vip/translate";
const MICROSOFT_GLOBAL_ENDPOINT: &str = "https://api.cognitive.microsofttranslator.com";
const MICROSOFT_CHINA_ENDPOINT: &str = "https://api.translator.azure.cn";
static BAIDU_SALT: AtomicU64 = AtomicU64::new(1_726_000_000);

fn microsoft_endpoint(cloud: MicrosoftCloud) -> &'static str {
    match cloud {
        MicrosoftCloud::Global => MICROSOFT_GLOBAL_ENDPOINT,
        MicrosoftCloud::China => MICROSOFT_CHINA_ENDPOINT,
    }
}

fn microsoft_translation_url(
    cloud: MicrosoftCloud,
    source_language: &str,
    target_language: &str,
) -> Result<reqwest::Url, AppError> {
    let mut url = reqwest::Url::parse(&format!("{}/translate", microsoft_endpoint(cloud)))
        .map_err(|_| internal_error("Microsoft Translator endpoint is invalid"))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("api-version", "3.0");
        query.append_pair("to", target_language);
        if !source_language.eq_ignore_ascii_case("auto") {
            query.append_pair("from", source_language);
        }
    }
    Ok(url)
}

fn microsoft_dictionary_url(
    cloud: MicrosoftCloud,
    source_language: &str,
    target_language: &str,
) -> Result<reqwest::Url, AppError> {
    let mut url = reqwest::Url::parse(&format!("{}/dictionary/lookup", microsoft_endpoint(cloud)))
        .map_err(|_| internal_error("Microsoft Dictionary endpoint is invalid"))?;
    url.query_pairs_mut()
        .append_pair("api-version", "3.0")
        .append_pair("from", source_language)
        .append_pair("to", target_language);
    Ok(url)
}

fn microsoft_dictionary_language(value: &str) -> &str {
    match value {
        "zh-CN" | "zh-Hans" => "zh-Hans",
        value => value,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DictionaryCapabilityArtifact {
    schema_version: u8,
    documentation_url: String,
    reviewed_at: String,
    pairs: Vec<DictionaryPair>,
}

#[derive(Deserialize)]
struct DictionaryPair {
    source: String,
    target: String,
}

fn microsoft_dictionary_pair_supported(source: &str, target: &str) -> bool {
    let Ok(artifact) = serde_json::from_str::<DictionaryCapabilityArtifact>(include_str!(
        "../../resources/providers/microsoft-dictionary-pairs-v1.json"
    )) else {
        return false;
    };
    artifact.schema_version == 1
        && artifact
            .documentation_url
            .starts_with("https://learn.microsoft.com/")
        && !artifact.reviewed_at.trim().is_empty()
        && artifact.pairs.iter().any(|pair| {
            pair.source.eq_ignore_ascii_case(source) && pair.target.eq_ignore_ascii_case(target)
        })
}

pub fn microsoft_dictionary_supported(source: &str, target: &str) -> bool {
    microsoft_dictionary_pair_supported(
        microsoft_dictionary_language(source),
        microsoft_dictionary_language(target),
    )
}

/// Selects one explicit provider per request. It never silently changes provider.
pub struct ProviderRouter {
    settings: Arc<dyn SettingsStore>,
    google: Arc<GoogleTranslationProvider>,
    baidu: Arc<BaiduTranslationProvider>,
    microsoft: Arc<MicrosoftTranslationProvider>,
}

impl ProviderRouter {
    pub fn new(
        settings: Arc<dyn SettingsStore>,
        credentials: Arc<ProviderCredentialStore>,
    ) -> Result<Self, AppError> {
        let google_credentials: Arc<dyn CredentialStore> = Arc::new(
            super::credentials::GoogleCredentialView::new(credentials.clone()),
        );
        Ok(Self {
            google: Arc::new(GoogleTranslationProvider::new(google_credentials)?),
            baidu: Arc::new(BaiduTranslationProvider::new(credentials.clone())?),
            microsoft: Arc::new(MicrosoftTranslationProvider::new(
                credentials,
                settings.clone(),
            )?),
            settings,
        })
    }

    fn selected(&self) -> Result<TranslationProviderId, AppError> {
        Ok(self.settings.load()?.translation_provider)
    }

    pub async fn test(&self, provider: TranslationProviderId) -> Result<(), AppError> {
        let request = TranslationRequest {
            selection_id: 0,
            text: "hello".to_owned(),
            example_sentence: None,
            source_language: "en".to_owned(),
            target_language: "zh-CN".to_owned(),
        };
        match provider {
            TranslationProviderId::Google => self.google.translate(&request).await,
            TranslationProviderId::Baidu => self.baidu.translate(&request).await,
            TranslationProviderId::Microsoft => self.microsoft.translate(&request).await,
        }
        .map(|_| ())
    }
}

#[async_trait]
impl TranslationProvider for ProviderRouter {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        match self.selected()? {
            TranslationProviderId::Google => self.google.translate(request).await,
            TranslationProviderId::Baidu => self.baidu.translate(request).await,
            TranslationProviderId::Microsoft => self.microsoft.translate(request).await,
        }
    }

    async fn supported_languages(&self) -> Result<Vec<LanguageCode>, AppError> {
        match self.selected()? {
            TranslationProviderId::Google => self.google.supported_languages().await,
            TranslationProviderId::Baidu => self.baidu.supported_languages().await,
            TranslationProviderId::Microsoft => self.microsoft.supported_languages().await,
        }
    }
}

pub struct BaiduTranslationProvider {
    client: Client,
    credentials: Arc<ProviderCredentialStore>,
    endpoint: String,
}

impl BaiduTranslationProvider {
    pub fn new(credentials: Arc<ProviderCredentialStore>) -> Result<Self, AppError> {
        Ok(Self {
            client: GoogleTranslationProvider::build_client()?,
            credentials,
            endpoint: BAIDU_ENDPOINT.to_owned(),
        })
    }

    fn credential(
        &self,
        field: ProviderSecretField,
        label: &'static str,
    ) -> Result<String, AppError> {
        self.credentials
            .get(TranslationProviderId::Baidu, field)?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::new(AppErrorCode::MissingCredential, label, false))
    }
}

#[derive(Deserialize)]
struct BaiduResponse {
    #[serde(default)]
    from: String,
    #[serde(default)]
    trans_result: Vec<BaiduTranslation>,
    error_code: Option<String>,
    #[serde(default)]
    error_msg: String,
}

#[derive(Deserialize)]
struct BaiduTranslation {
    dst: String,
}

fn baidu_language(value: &str) -> &str {
    match value {
        "zh-CN" | "zh-Hans" => "zh",
        "zh-TW" | "zh-Hant" => "cht",
        value => value,
    }
}

#[async_trait]
impl TranslationProvider for BaiduTranslationProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        request.validate()?;
        let app_id = self.credential(ProviderSecretField::AppId, "Add your Baidu APP ID.")?;
        let api_key = self.credential(ProviderSecretField::ApiKey, "Add your Baidu secret key.")?;
        let salt = BAIDU_SALT.fetch_add(1, Ordering::Relaxed).to_string();
        let sign = format!(
            "{:x}",
            md5::compute(format!("{app_id}{}{salt}{api_key}", request.text))
        );
        let params = [
            ("q", request.text.as_str()),
            ("from", baidu_language(&request.source_language)),
            ("to", baidu_language(&request.target_language)),
            ("appid", app_id.as_str()),
            ("salt", salt.as_str()),
            ("sign", sign.as_str()),
        ];
        let response = self
            .client
            .post(&self.endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|error| map_transport_error_named(&error, "Baidu Translate"))?;
        let status = response.status();
        let body = read_bounded_body(response).await?;
        if !status.is_success() {
            return Err(map_generic_http_error(status, "Baidu Translate"));
        }
        let envelope: BaiduResponse = serde_json::from_slice(&body)
            .map_err(|_| internal_error("Baidu Translate returned an invalid response"))?;
        if let Some(code) = envelope.error_code.as_deref() {
            return Err(map_baidu_error(code, &envelope.error_msg));
        }
        let translated_text = envelope
            .trans_result
            .into_iter()
            .map(|item| item.dst)
            .collect::<Vec<_>>()
            .join("\n");
        if translated_text.trim().is_empty() {
            return Err(internal_error("Baidu Translate returned no translation"));
        }
        let detected = (!envelope.from.trim().is_empty()).then_some(envelope.from);
        Ok(TranslationResult {
            selection_id: request.selection_id,
            translated_text: translated_text.clone(),
            detected_source_language: detected.clone(),
            effective_source_language: if request.source_language.eq_ignore_ascii_case("auto") {
                detected.unwrap_or_else(|| "auto".to_owned())
            } else {
                request.source_language.clone()
            },
            target_language: request.target_language.clone(),
            part_of_speech: None,
            senses: vec![TranslationSense {
                text: translated_text.clone(),
                part_of_speech: None,
                rank: 0,
                is_primary: true,
                confidence: None,
            }],
        })
    }

    async fn supported_languages(&self) -> Result<Vec<LanguageCode>, AppError> {
        Ok(vec!["auto", "en", "zh-CN", "ja", "ko", "fr", "de", "es"]
            .into_iter()
            .map(str::to_owned)
            .collect())
    }
}

fn map_baidu_error(code: &str, _provider_message: &str) -> AppError {
    match code {
        "52001" => AppError::new(AppErrorCode::Timeout, "Baidu Translate timed out.", true),
        "54003" => AppError::new(
            AppErrorCode::QuotaExceeded,
            "Baidu request frequency is limited.",
            true,
        ),
        "54004" => AppError::new(
            AppErrorCode::BillingRequired,
            "Baidu Translate balance is insufficient.",
            false,
        ),
        "58001" => AppError::new(
            AppErrorCode::InvalidLanguagePair,
            "Baidu rejected the language pair.",
            false,
        ),
        "52003" | "54001" => AppError::new(
            AppErrorCode::InvalidCredential,
            "Baidu rejected the credentials.",
            false,
        ),
        _ => AppError::new(
            AppErrorCode::ServiceUnavailable,
            "Baidu Translate rejected the request.",
            false,
        ),
    }
}

pub struct MicrosoftTranslationProvider {
    client: Client,
    credentials: Arc<ProviderCredentialStore>,
    settings: Arc<dyn SettingsStore>,
}

impl MicrosoftTranslationProvider {
    pub fn new(
        credentials: Arc<ProviderCredentialStore>,
        settings: Arc<dyn SettingsStore>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            client: GoogleTranslationProvider::build_client()?,
            credentials,
            settings,
        })
    }

    async fn dictionary_senses(
        &self,
        key: &str,
        cloud: MicrosoftCloud,
        region: Option<&str>,
        source_language: &str,
        target_language: &str,
        text: &str,
    ) -> Vec<TranslationSense> {
        if text.chars().count() > 100 {
            return Vec::new();
        }
        let source = microsoft_dictionary_language(source_language);
        let target = microsoft_dictionary_language(target_language);
        if !microsoft_dictionary_pair_supported(source, target) {
            return Vec::new();
        }
        let Ok(url) = microsoft_dictionary_url(cloud, source, target) else {
            return Vec::new();
        };
        let mut outbound = self
            .client
            .post(url)
            .header("Ocp-Apim-Subscription-Key", key)
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&[MicrosoftRequest { text }]);
        if let Some(region) = region.filter(|value| !value.trim().is_empty()) {
            outbound = outbound.header("Ocp-Apim-Subscription-Region", region);
        }
        let Ok(response) = outbound.send().await else {
            return Vec::new();
        };
        if !response.status().is_success() {
            return Vec::new();
        }
        let Ok(body) = read_bounded_body(response).await else {
            return Vec::new();
        };
        parse_microsoft_dictionary(&body).unwrap_or_default()
    }
}

#[derive(Serialize)]
struct MicrosoftRequest<'a> {
    text: &'a str,
}

#[derive(Deserialize)]
struct MicrosoftResponse {
    #[serde(default)]
    detected_language: Option<MicrosoftDetected>,
    translations: Vec<MicrosoftTranslation>,
}
#[derive(Deserialize)]
struct MicrosoftDetected {
    language: String,
}
#[derive(Deserialize)]
struct MicrosoftTranslation {
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MicrosoftDictionaryResponse {
    translations: Vec<MicrosoftDictionaryTranslation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MicrosoftDictionaryTranslation {
    display_target: String,
    #[serde(default)]
    prefix_word: String,
    #[serde(default)]
    pos_tag: String,
    #[serde(default)]
    confidence: Option<f64>,
}

fn microsoft_pos(value: &str) -> Option<PartOfSpeech> {
    match value.trim().to_ascii_uppercase().as_str() {
        "ADJ" => Some(PartOfSpeech::Adjective),
        "ADV" => Some(PartOfSpeech::Adverb),
        "CONJ" => Some(PartOfSpeech::Conjunction),
        "DET" => Some(PartOfSpeech::Determiner),
        "NOUN" => Some(PartOfSpeech::Noun),
        "PREP" => Some(PartOfSpeech::Preposition),
        "PRON" => Some(PartOfSpeech::Pronoun),
        "VERB" | "MODAL" => Some(PartOfSpeech::Verb),
        _ => None,
    }
}

fn parse_microsoft_dictionary(body: &[u8]) -> Result<Vec<TranslationSense>, AppError> {
    let entries: Vec<MicrosoftDictionaryResponse> = serde_json::from_slice(body)
        .map_err(|_| internal_error("Microsoft Dictionary returned an invalid response"))?;
    let mut senses = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for translation in entries.into_iter().flat_map(|entry| entry.translations) {
        let text = format!("{}{}", translation.prefix_word, translation.display_target)
            .trim()
            .to_owned();
        let pos = microsoft_pos(&translation.pos_tag);
        if text.is_empty() || !seen.insert((text.to_lowercase(), pos)) {
            continue;
        }
        senses.push(TranslationSense {
            text,
            part_of_speech: pos,
            rank: 0,
            is_primary: false,
            confidence: translation.confidence.filter(|value| value.is_finite()),
        });
    }
    senses.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.text.cmp(&right.text))
            .then_with(|| left.part_of_speech.cmp(&right.part_of_speech))
    });
    if senses.len() > 32 {
        eprintln!(
            "lexical sense overflow: provider=microsoft retained=32 discarded={}",
            senses.len() - 32
        );
        senses.truncate(32);
    }
    for (rank, sense) in senses.iter_mut().enumerate() {
        sense.rank = rank.min(31) as u8;
    }
    Ok(senses)
}

#[async_trait]
impl TranslationProvider for MicrosoftTranslationProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        request.validate()?;
        let key = self
            .credentials
            .get(
                TranslationProviderId::Microsoft,
                ProviderSecretField::ApiKey,
            )?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AppError::new(
                    AppErrorCode::MissingCredential,
                    "Add a Microsoft Translator key.",
                    false,
                )
            })?;
        let settings = self.settings.load()?;
        let url = microsoft_translation_url(
            settings.microsoft_cloud,
            &request.source_language,
            &request.target_language,
        )?;
        let mut outbound = self
            .client
            .post(url)
            .header("Ocp-Apim-Subscription-Key", key.as_str())
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&[MicrosoftRequest {
                text: &request.text,
            }]);
        if let Some(region) = settings
            .microsoft_region
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            outbound = outbound.header("Ocp-Apim-Subscription-Region", region);
        }
        let response = outbound
            .send()
            .await
            .map_err(|error| map_transport_error_named(&error, "Microsoft Translator"))?;
        let status = response.status();
        let body = read_bounded_body(response).await?;
        if !status.is_success() {
            return Err(map_generic_http_error(status, "Microsoft Translator"));
        }
        let first: MicrosoftResponse = serde_json::from_slice::<Vec<MicrosoftResponse>>(&body)
            .map_err(|_| internal_error("Microsoft Translator returned an invalid response"))?
            .into_iter()
            .next()
            .ok_or_else(|| internal_error("Microsoft Translator returned no translation"))?;
        let translated_text = first
            .translations
            .into_iter()
            .next()
            .map(|item| item.text)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| internal_error("Microsoft Translator returned no translation"))?;
        let detected = first.detected_language.map(|item| item.language);
        let effective_source_language = if request.source_language.eq_ignore_ascii_case("auto") {
            detected.clone().unwrap_or_else(|| "auto".to_owned())
        } else {
            request.source_language.clone()
        };
        let mut alternatives = self
            .dictionary_senses(
                &key,
                settings.microsoft_cloud,
                settings.microsoft_region.as_deref(),
                &effective_source_language,
                &request.target_language,
                &request.text,
            )
            .await;
        let primary_pos = alternatives
            .iter()
            .find(|sense| {
                sense
                    .text
                    .trim()
                    .eq_ignore_ascii_case(translated_text.trim())
            })
            .and_then(|sense| sense.part_of_speech);
        alternatives.retain(|sense| {
            !sense
                .text
                .trim()
                .eq_ignore_ascii_case(translated_text.trim())
                || sense.part_of_speech != primary_pos
        });
        let mut senses = vec![TranslationSense {
            text: translated_text.clone(),
            part_of_speech: primary_pos,
            rank: 0,
            is_primary: true,
            confidence: None,
        }];
        for mut sense in alternatives.into_iter().take(31) {
            sense.rank = senses.len() as u8;
            senses.push(sense);
        }
        Ok(TranslationResult {
            selection_id: request.selection_id,
            translated_text: translated_text.clone(),
            detected_source_language: detected.clone(),
            effective_source_language,
            target_language: request.target_language.clone(),
            part_of_speech: primary_pos,
            senses,
        })
    }

    async fn supported_languages(&self) -> Result<Vec<LanguageCode>, AppError> {
        Ok(vec!["auto", "en", "zh-CN", "ja", "ko", "fr", "de", "es"]
            .into_iter()
            .map(str::to_owned)
            .collect())
    }
}

fn map_transport_error_named(error: &reqwest::Error, provider: &'static str) -> AppError {
    let (code, suffix) = if error.is_timeout() {
        (AppErrorCode::Timeout, "timed out")
    } else if error.is_connect() {
        (AppErrorCode::Offline, "could not be reached")
    } else {
        (
            AppErrorCode::ServiceUnavailable,
            "is temporarily unavailable",
        )
    };
    AppError::new(code, format!("{provider} {suffix}."), true)
}

fn map_generic_http_error(status: StatusCode, provider: &'static str) -> AppError {
    if status == StatusCode::UNAUTHORIZED {
        AppError::new(
            AppErrorCode::InvalidCredential,
            format!("{provider} rejected the credential."),
            false,
        )
    } else if status == StatusCode::FORBIDDEN {
        AppError::new(
            AppErrorCode::ApiRestricted,
            format!("{provider} rejected this account or region."),
            false,
        )
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        AppError::new(
            AppErrorCode::QuotaExceeded,
            format!("{provider} quota is exhausted."),
            false,
        )
    } else if status == StatusCode::BAD_REQUEST {
        AppError::new(
            AppErrorCode::InvalidLanguagePair,
            format!("{provider} rejected the language pair."),
            false,
        )
    } else if status.is_server_error() {
        AppError::new(
            AppErrorCode::ServiceUnavailable,
            format!("{provider} is temporarily unavailable."),
            true,
        )
    } else {
        AppError::new(
            AppErrorCode::Internal,
            format!("{provider} rejected the request."),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
    };

    use super::*;

    #[test]
    fn microsoft_dictionary_parses_ranked_distinct_senses_and_pos() {
        let body = r#"[{
          "normalizedSource":"rationality",
          "displaySource":"rationality",
          "translations":[
            {"normalizedTarget":"理性","displayTarget":"理性","posTag":"NOUN","confidence":0.91,"prefixWord":"","backTranslations":[]},
            {"normalizedTarget":"合理","displayTarget":"合理","posTag":"ADJ","confidence":0.72,"prefixWord":"","backTranslations":[]},
            {"normalizedTarget":"理性","displayTarget":"理性","posTag":"NOUN","confidence":0.40,"prefixWord":"","backTranslations":[]}
          ]
        }]"#;
        let senses = parse_microsoft_dictionary(body.as_bytes()).expect("dictionary response");
        assert_eq!(senses.len(), 2);
        assert_eq!(senses[0].text, "理性");
        assert_eq!(senses[0].part_of_speech, Some(PartOfSpeech::Noun));
        assert_eq!(senses[1].part_of_speech, Some(PartOfSpeech::Adjective));
        assert!(microsoft_dictionary_pair_supported("en", "zh-Hans"));
        assert!(!microsoft_dictionary_pair_supported("en", "ja"));
    }

    #[test]
    fn microsoft_dictionary_records_a_bounded_ranked_result() {
        let translations = (0..35)
            .map(|index| {
                format!(
                    r#"{{"normalizedTarget":"sense-{index}","displayTarget":"sense-{index}","posTag":"NOUN","confidence":0.5,"prefixWord":"","backTranslations":[]}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(
            r#"[{{"normalizedSource":"word","displaySource":"word","translations":[{translations}]}}]"#
        );
        let senses = parse_microsoft_dictionary(body.as_bytes()).expect("dictionary response");
        assert_eq!(senses.len(), 32);
        assert_eq!(senses.first().map(|sense| sense.rank), Some(0));
        assert_eq!(senses.last().map(|sense| sense.rank), Some(31));
    }

    struct StaticCredentialStore(String);

    #[async_trait]
    impl CredentialStore for StaticCredentialStore {
        fn set_api_key(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }

        fn get_api_key(&self) -> Result<Option<String>, AppError> {
            Ok(Some(self.0.clone()))
        }

        async fn test_api_key(&self) -> Result<(), AppError> {
            Ok(())
        }

        fn remove_api_key(&self) -> Result<(), AppError> {
            Ok(())
        }
    }

    struct MockResponse {
        status: &'static str,
        body: &'static str,
        delay: Duration,
    }

    fn spawn_mock_server(
        responses: Vec<MockResponse>,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let endpoint = format!("http://{}", listener.local_addr().expect("mock address"));
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let request = read_http_request(&mut stream);
                sender.send(request).expect("record mock request");
                thread::sleep(response.delay);
                let wire = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    response.body.len(),
                    response.body
                );
                let _ = stream.write_all(wire.as_bytes());
            }
        });
        (endpoint, receiver, handle)
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set read timeout");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        let mut expected_length = None;
        loop {
            let count = stream.read(&mut buffer).expect("read mock request");
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if expected_length.is_none() {
                if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers =
                        String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
                    let content_length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    expected_length = Some(header_end + 4 + content_length);
                }
            }
            if expected_length.is_some_and(|length| bytes.len() >= length) {
                break;
            }
        }
        String::from_utf8(bytes).expect("UTF-8 mock request")
    }

    fn request(source: &str) -> TranslationRequest {
        TranslationRequest {
            selection_id: 41,
            text: "Fish & chips".to_owned(),
            example_sentence: None,
            source_language: source.to_owned(),
            target_language: "fr".to_owned(),
        }
    }

    #[test]
    fn auto_source_is_omitted_and_key_is_not_part_of_payload() {
        let payload = GoogleTranslateRequest {
            q: "secret selected text",
            source: None,
            target: "fr",
            format: "text",
        };
        let json = serde_json::to_value(payload).expect("payload serializes");
        assert_eq!(json["q"], "secret selected text");
        assert_eq!(json["format"], "text");
        assert!(json.get("source").is_none());
        assert!(json.get("apiKey").is_none());
        assert!(json.get("key").is_none());
    }

    #[test]
    fn translation_response_preserves_correlation_and_decodes_entities() {
        let body = br#"{"data":{"translations":[{"translatedText":"Poisson &amp; frites","detectedSourceLanguage":"en"}]}}"#;
        let result = parse_translation_response(&request("auto"), body).expect("valid response");
        assert_eq!(result.selection_id, 41);
        assert_eq!(result.translated_text, "Poisson & frites");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
        assert_eq!(result.effective_source_language, "en");
        assert_eq!(result.target_language, "fr");
    }

    #[test]
    fn explicit_source_wins_over_provider_detection() {
        let body = br#"{"data":{"translations":[{"translatedText":"Bonjour","detectedSourceLanguage":"de"}]}}"#;
        let result = parse_translation_response(&request("en"), body).expect("valid response");
        assert_eq!(result.effective_source_language, "en");
        assert_eq!(result.detected_source_language.as_deref(), Some("de"));
    }

    #[test]
    fn malformed_or_empty_success_bodies_are_rejected_safely() {
        assert_eq!(
            parse_translation_response(&request("auto"), b"selected text")
                .unwrap_err()
                .code,
            AppErrorCode::Internal
        );
        assert_eq!(
            parse_translation_response(&request("auto"), br#"{"data":{"translations":[]}}"#)
                .unwrap_err()
                .code,
            AppErrorCode::Internal
        );
    }

    #[test]
    fn languages_are_filtered_sorted_and_deduplicated() {
        let body = br#"{"data":{"languages":[{"language":"zh"},{"language":""},{"language":"en"},{"language":"zh"}]}}"#;
        assert_eq!(
            parse_languages_response(body).expect("valid languages"),
            vec!["en", "zh"]
        );
    }

    #[test]
    fn provider_failures_map_to_stable_recovery_categories() {
        let cases = [
            (
                StatusCode::UNAUTHORIZED,
                br#"{"error":{"status":"UNAUTHENTICATED"}}"#.as_slice(),
                AppErrorCode::InvalidCredential,
                false,
            ),
            (
                StatusCode::BAD_REQUEST,
                br#"{"error":{"message":"API key not valid. Please pass a valid API key."}}"#
                    .as_slice(),
                AppErrorCode::InvalidCredential,
                false,
            ),
            (
                StatusCode::FORBIDDEN,
                br#"{"error":{"details":[{"reason":"API_KEY_SERVICE_BLOCKED"}]}}"#.as_slice(),
                AppErrorCode::ApiRestricted,
                false,
            ),
            (
                StatusCode::FORBIDDEN,
                br#"{"error":{"message":"Billing is disabled"}}"#.as_slice(),
                AppErrorCode::BillingRequired,
                false,
            ),
            (
                StatusCode::TOO_MANY_REQUESTS,
                b"body deliberately ignored".as_slice(),
                AppErrorCode::QuotaExceeded,
                false,
            ),
            (
                StatusCode::FORBIDDEN,
                br#"{"error":{"message":"Daily Limit Exceeded"}}"#.as_slice(),
                AppErrorCode::QuotaExceeded,
                false,
            ),
            (
                StatusCode::FORBIDDEN,
                br#"{"error":{"message":"User Rate Limit Exceeded"}}"#.as_slice(),
                AppErrorCode::QuotaExceeded,
                false,
            ),
            (
                StatusCode::BAD_REQUEST,
                br#"{"error":{"status":"INVALID_ARGUMENT"}}"#.as_slice(),
                AppErrorCode::InvalidLanguagePair,
                false,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                b"provider body deliberately ignored".as_slice(),
                AppErrorCode::ServiceUnavailable,
                true,
            ),
        ];

        for (status, body, code, retryable) in cases {
            let error = map_http_error(status, body);
            assert_eq!(error.code, code);
            assert_eq!(error.retryable, retryable);
            assert!(!error.message.contains("deliberately ignored"));
        }
    }

    #[tokio::test]
    async fn local_server_observes_post_header_only_key_and_auto_source_omission() {
        let response = r#"{"data":{"translations":[{"translatedText":"Poisson","detectedSourceLanguage":"en"}]}}"#;
        let (endpoint, requests, server) = spawn_mock_server(vec![MockResponse {
            status: "200 OK",
            body: response,
            delay: Duration::ZERO,
        }]);
        let api_key = "synthetic-local-key";
        let provider = GoogleTranslationProvider::with_test_endpoints(
            Client::builder()
                .timeout(Duration::from_secs(1))
                .build()
                .expect("test client"),
            Arc::new(StaticCredentialStore(api_key.to_owned())),
            format!("{endpoint}/translate"),
            format!("{endpoint}/languages"),
            1,
        );

        provider
            .translate(&request("auto"))
            .await
            .expect("mock translation");
        let raw = requests
            .recv_timeout(Duration::from_secs(1))
            .expect("captured request");
        server.join().expect("mock server");
        let (headers, body) = raw.split_once("\r\n\r\n").expect("HTTP request");
        let mut lines = headers.lines();
        let request_line = lines.next().expect("request line");
        assert_eq!(request_line, "POST /translate HTTP/1.1");
        assert!(!request_line.contains(api_key));
        let key_headers: Vec<_> = lines
            .filter(|line| line.to_ascii_lowercase().starts_with("x-goog-api-key:"))
            .collect();
        assert_eq!(key_headers.len(), 1);
        assert_eq!(
            key_headers[0].to_ascii_lowercase(),
            format!("x-goog-api-key: {api_key}")
        );
        assert!(!headers.to_ascii_lowercase().contains("authorization:"));
        assert!(!body.contains(api_key));
        let payload: serde_json::Value = serde_json::from_str(body).expect("JSON payload");
        assert_eq!(payload["q"], "Fish & chips");
        assert_eq!(payload["target"], "fr");
        assert!(payload.get("source").is_none());
        assert!(payload.get("key").is_none());
    }

    #[tokio::test]
    async fn local_server_observes_bounded_retry_count() {
        let responses = (0..DEFAULT_MAX_ATTEMPTS)
            .map(|_| MockResponse {
                status: "503 Service Unavailable",
                body: r#"{"error":{"status":"UNAVAILABLE"}}"#,
                delay: Duration::ZERO,
            })
            .collect();
        let (endpoint, requests, server) = spawn_mock_server(responses);
        let provider = GoogleTranslationProvider::with_test_endpoints(
            Client::builder()
                .timeout(Duration::from_secs(1))
                .build()
                .expect("test client"),
            Arc::new(StaticCredentialStore("synthetic-key".to_owned())),
            format!("{endpoint}/translate"),
            format!("{endpoint}/languages"),
            DEFAULT_MAX_ATTEMPTS,
        );

        let error = provider.translate(&request("en")).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::ServiceUnavailable);
        assert!(error.retryable);
        for _ in 0..DEFAULT_MAX_ATTEMPTS {
            requests
                .recv_timeout(Duration::from_secs(1))
                .expect("captured retry");
        }
        assert!(requests.try_recv().is_err());
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn local_server_delay_maps_to_timeout() {
        let response = r#"{"data":{"translations":[{"translatedText":"Poisson","detectedSourceLanguage":"en"}]}}"#;
        let (endpoint, requests, server) = spawn_mock_server(vec![MockResponse {
            status: "200 OK",
            body: response,
            delay: Duration::from_millis(200),
        }]);
        let provider = GoogleTranslationProvider::with_test_endpoints(
            Client::builder()
                .timeout(Duration::from_millis(30))
                .build()
                .expect("test client"),
            Arc::new(StaticCredentialStore("synthetic-key".to_owned())),
            format!("{endpoint}/translate"),
            format!("{endpoint}/languages"),
            1,
        );

        let error = provider.translate(&request("en")).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::Timeout);
        assert!(error.retryable);
        requests
            .recv_timeout(Duration::from_secs(1))
            .expect("captured timed-out request");
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn closed_local_port_maps_to_offline() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve local port");
        let endpoint = format!("http://{}", listener.local_addr().expect("local address"));
        drop(listener);
        let provider = GoogleTranslationProvider::with_test_endpoints(
            Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_millis(100))
                .timeout(Duration::from_millis(200))
                .build()
                .expect("test client"),
            Arc::new(StaticCredentialStore("synthetic-key".to_owned())),
            format!("{endpoint}/translate"),
            format!("{endpoint}/languages"),
            1,
        );

        let error = provider.translate(&request("en")).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::Offline);
        assert!(error.retryable);
    }

    #[test]
    fn test_constructor_bounds_attempt_count() {
        struct UnusedCredentialStore;
        #[async_trait]
        impl CredentialStore for UnusedCredentialStore {
            fn set_api_key(&self, _: &str) -> Result<(), AppError> {
                Ok(())
            }
            fn get_api_key(&self) -> Result<Option<String>, AppError> {
                Ok(None)
            }
            async fn test_api_key(&self) -> Result<(), AppError> {
                Ok(())
            }
            fn remove_api_key(&self) -> Result<(), AppError> {
                Ok(())
            }
        }

        let provider = GoogleTranslationProvider::with_test_endpoints(
            Client::new(),
            Arc::new(UnusedCredentialStore),
            "http://127.0.0.1/translate",
            "http://127.0.0.1/languages",
            0,
        );
        assert_eq!(provider.max_attempts, 1);
    }

    #[test]
    fn china_provider_language_and_endpoint_profiles_are_explicit() {
        assert_eq!(baidu_language("zh-CN"), "zh");
        assert_eq!(baidu_language("zh-TW"), "cht");
        assert_eq!(baidu_language("en"), "en");
        assert_eq!(
            microsoft_endpoint(MicrosoftCloud::China),
            "https://api.translator.azure.cn"
        );
        assert_eq!(
            microsoft_endpoint(MicrosoftCloud::Global),
            "https://api.cognitive.microsofttranslator.com"
        );
        let url = microsoft_translation_url(MicrosoftCloud::China, "en&category=x", "zh-CN")
            .expect("encoded Microsoft URL");
        assert_eq!(url.host_str(), Some("api.translator.azure.cn"));
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![
                ("api-version".into(), "3.0".into()),
                ("to".into(), "zh-CN".into()),
                ("from".into(), "en&category=x".into()),
            ]
        );
    }

    #[test]
    fn microsoft_and_baidu_success_envelopes_are_provider_local() {
        let microsoft: Vec<MicrosoftResponse> = serde_json::from_str(
            r#"[{"detectedLanguage":{"language":"en"},"translations":[{"text":"你好"}]}]"#,
        )
        .expect("Microsoft envelope");
        assert_eq!(microsoft[0].translations[0].text, "你好");
        let baidu: BaiduResponse = serde_json::from_str(
            r#"{"from":"en","to":"zh","trans_result":[{"src":"hello","dst":"你好"}]}"#,
        )
        .expect("Baidu envelope");
        assert_eq!(baidu.trans_result[0].dst, "你好");
    }

    /// Networks that reach Google only through a local proxy expose whether the
    /// production client performs proxy discovery at all. Run it with the proxy
    /// environment variables removed, so only the operating system's own
    /// configuration can satisfy the request:
    ///
    /// ```sh
    /// env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
    ///   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture reaches_google
    /// ```
    #[tokio::test]
    #[ignore = "manual network fixture: contacts the real Google endpoint"]
    async fn production_client_reaches_google_without_proxy_environment_variables() {
        let client = GoogleTranslationProvider::build_client().expect("production client");
        let response = client
            .get(format!("{LANGUAGES_ENDPOINT}?key=deliberately-invalid"))
            .send()
            .await;

        match response {
            // An invalid key is rejected by Google, which proves the endpoint
            // was reached rather than blocked.
            Ok(response) => assert!(
                response.status().is_client_error(),
                "unexpected status {}",
                response.status()
            ),
            Err(error) => panic!("the endpoint was unreachable ({error})"),
        }
    }
}
