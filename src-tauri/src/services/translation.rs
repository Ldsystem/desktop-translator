//! Google Cloud Translation Basic REST adapter.
//!
//! This module intentionally emits no request, response, credential, or text logs.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    contracts::{
        AppError, AppErrorCode, LanguageCode, TranslationRequest, TranslationResult,
        ValidateContract,
    },
    services::{CredentialStore, TranslationProvider},
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
    /// Creates a provider with a bounded request timeout and one retry.
    pub fn new(credentials: Arc<dyn CredentialStore>) -> Result<Self, AppError> {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .https_only(true)
            .build()
            .map_err(|_| internal_error("translation client could not be initialized"))?;

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
        translated_text,
        detected_source_language: detected,
        effective_source_language,
        target_language: request.target_language.clone(),
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
    };

    use super::*;

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
}
