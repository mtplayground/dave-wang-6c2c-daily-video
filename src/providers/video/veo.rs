#![allow(dead_code)]

use std::{
    fmt::{self, Display},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use super::{
    VideoClip, VideoGenerationRequest, VideoJob, VideoJobStatus, VideoProvider, VideoProviderError,
    VideoProviderKind,
};

pub const DEFAULT_VEO_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const DEFAULT_VEO_MODEL: &str = "veo-3.1-generate-preview";

#[derive(Clone)]
pub struct VeoVideoProvider {
    api_key: String,
    model: String,
    base_url: String,
    http: Arc<dyn VeoHttpClient>,
    retry_policy: RetryPolicy,
}

impl VeoVideoProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: DEFAULT_VEO_MODEL.to_owned(),
            base_url: DEFAULT_VEO_BASE_URL.to_owned(),
            http: Arc::new(ReqwestVeoHttpClient::default()),
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn with_options(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        http: Arc<dyn VeoHttpClient>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: trim_trailing_slashes(base_url.into()),
            http,
            retry_policy,
        }
    }

    pub async fn wait_until_ready(
        &self,
        job: &VideoJob,
    ) -> Result<VideoJobStatus, VideoProviderError> {
        loop {
            match self.poll_job(job).await? {
                VideoJobStatus::Pending | VideoJobStatus::Running => {
                    sleep(self.retry_policy.poll_interval).await;
                }
                terminal => return Ok(terminal),
            }
        }
    }

    fn generate_url(&self) -> String {
        format!(
            "{}/models/{}:predictLongRunning",
            self.base_url,
            self.model.trim_matches('/')
        )
    }

    fn operation_url(&self, operation_name: &str) -> String {
        if operation_name.starts_with("http://") || operation_name.starts_with("https://") {
            operation_name.to_owned()
        } else {
            format!(
                "{}/{}",
                self.base_url,
                operation_name.trim_start_matches('/')
            )
        }
    }

    async fn send_with_retries(
        &self,
        request: HttpRequest,
        context: ErrorContext,
    ) -> Result<HttpResponse, VideoProviderError> {
        let mut attempts = 0;
        let mut delay = self.retry_policy.initial_backoff;

        loop {
            attempts += 1;
            let result = self.http.send(request.clone()).await;

            match result {
                Ok(response) if response.is_success() => return Ok(response),
                Ok(response)
                    if response.is_transient() && attempts < self.retry_policy.max_attempts =>
                {
                    sleep(delay).await;
                    delay = self.retry_policy.next_delay(delay);
                }
                Ok(response) => {
                    return Err(context.error(format!(
                        "Gemini/Veo returned HTTP {}: {}",
                        response.status,
                        response.text_lossy()
                    )));
                }
                Err(error) if error.transient && attempts < self.retry_policy.max_attempts => {
                    sleep(delay).await;
                    delay = self.retry_policy.next_delay(delay);
                }
                Err(error) => return Err(context.error(error.to_string())),
            }
        }
    }

    async fn get_operation(
        &self,
        operation_name: &str,
    ) -> Result<VeoOperation, VideoProviderError> {
        let response = self
            .send_with_retries(
                HttpRequest::get(self.operation_url(operation_name)).with_api_key(&self.api_key),
                ErrorContext::Poll,
            )
            .await?;

        response.decode_json(ErrorContext::Poll)
    }

    async fn download_uri(&self, uri: &str) -> Result<VideoClip, VideoProviderError> {
        let response = self
            .send_with_retries(
                HttpRequest::get(uri.to_owned()).with_api_key(&self.api_key),
                ErrorContext::Download,
            )
            .await?;

        let content_type = response
            .header("content-type")
            .unwrap_or("video/mp4")
            .to_owned();
        let file_extension = file_extension_for_content_type(&content_type).to_owned();

        Ok(VideoClip {
            bytes: response.body,
            content_type,
            file_extension,
        })
    }
}

#[async_trait]
impl VideoProvider for VeoVideoProvider {
    async fn submit_prompt(
        &self,
        request: VideoGenerationRequest,
    ) -> Result<VideoJob, VideoProviderError> {
        let body = serde_json::to_vec(&VeoGenerateRequest::from(request)).map_err(|error| {
            VideoProviderError::SubmitFailed {
                message: format!("failed to encode request JSON: {error}"),
            }
        })?;

        let response = self
            .send_with_retries(
                HttpRequest::post_json(self.generate_url(), body).with_api_key(&self.api_key),
                ErrorContext::Submit,
            )
            .await?;
        let operation: VeoOperation = response.decode_json(ErrorContext::Submit)?;

        if operation.name.trim().is_empty() {
            return Err(VideoProviderError::SubmitFailed {
                message: "Gemini/Veo response did not include an operation name".to_owned(),
            });
        }

        Ok(VideoJob {
            provider: VideoProviderKind::GeminiVeo,
            provider_job_id: operation.name,
        })
    }

    async fn poll_job(&self, job: &VideoJob) -> Result<VideoJobStatus, VideoProviderError> {
        if job.provider != VideoProviderKind::GeminiVeo {
            return Err(VideoProviderError::PollFailed {
                message: format!("cannot poll {} job with Gemini/Veo provider", job.provider),
            });
        }

        let operation = self.get_operation(&job.provider_job_id).await?;

        if let Some(error) = operation.error {
            return Ok(VideoJobStatus::Failed {
                message: error.message.unwrap_or_else(|| {
                    error
                        .code
                        .map(|code| format!("Gemini/Veo operation failed with code {code}"))
                        .unwrap_or_else(|| "Gemini/Veo operation failed".to_owned())
                }),
            });
        }

        if operation.done.unwrap_or(false) {
            Ok(VideoJobStatus::Complete)
        } else {
            Ok(VideoJobStatus::Running)
        }
    }

    async fn download_clip(&self, job: &VideoJob) -> Result<VideoClip, VideoProviderError> {
        if job.provider != VideoProviderKind::GeminiVeo {
            return Err(VideoProviderError::DownloadFailed {
                message: format!(
                    "cannot download {} job with Gemini/Veo provider",
                    job.provider
                ),
            });
        }

        let operation = self.get_operation(&job.provider_job_id).await?;

        if let Some(error) = operation.error {
            return Err(VideoProviderError::DownloadFailed {
                message: error
                    .message
                    .unwrap_or_else(|| "Gemini/Veo operation failed".to_owned()),
            });
        }

        if !operation.done.unwrap_or(false) {
            return Err(VideoProviderError::DownloadFailed {
                message: "Gemini/Veo operation is not complete".to_owned(),
            });
        }

        let uri = operation
            .video_uri()
            .ok_or_else(|| VideoProviderError::DownloadFailed {
                message: "Gemini/Veo operation completed without a video download URI".to_owned(),
            })?;

        self.download_uri(uri).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub poll_interval: Duration,
}

impl RetryPolicy {
    pub fn no_delay(max_attempts: u8) -> Self {
        Self {
            max_attempts,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            poll_interval: Duration::ZERO,
        }
    }

    fn next_delay(self, current: Duration) -> Duration {
        if current.is_zero() {
            return current;
        }

        current.saturating_mul(2).min(self.max_backoff)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(2),
            poll_interval: Duration::from_secs(10),
        }
    }
}

#[async_trait]
pub trait VeoHttpClient: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, VeoHttpError>;
}

#[derive(Default)]
struct ReqwestVeoHttpClient {
    client: reqwest::Client,
}

#[async_trait]
impl VeoHttpClient for ReqwestVeoHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, VeoHttpError> {
        let mut builder = match request.method {
            HttpMethod::Get => self.client.get(&request.url),
            HttpMethod::Post => self.client.post(&request.url),
        };

        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }

        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let response = builder.send().await.map_err(VeoHttpError::from_reqwest)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(VeoHttpError::from_reqwest)?
            .to_vec();

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn get(url: String) -> Self {
        Self {
            method: HttpMethod::Get,
            url,
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn post_json(url: String, body: Vec<u8>) -> Self {
        Self {
            method: HttpMethod::Post,
            url,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: Some(body),
        }
    }

    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.headers
            .push(("x-goog-api-key".to_owned(), api_key.to_owned()));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(status: u16, value: serde_json::Value) -> Result<Self, serde_json::Error> {
        Ok(Self {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: serde_json::to_vec(&value)?,
        })
    }

    pub fn bytes(status: u16, content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), content_type.into())],
            body,
        }
    }

    fn is_success(&self) -> bool {
        (200..=299).contains(&self.status)
    }

    fn is_transient(&self) -> bool {
        matches!(self.status, 408 | 409 | 425 | 429 | 500..=599)
    }

    fn decode_json<T: for<'de> Deserialize<'de>>(
        &self,
        context: ErrorContext,
    ) -> Result<T, VideoProviderError> {
        serde_json::from_slice(&self.body).map_err(|error| {
            context.error(format!(
                "failed to decode Gemini/Veo JSON response: {error}"
            ))
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VeoHttpError {
    pub message: String,
    pub transient: bool,
}

impl VeoHttpError {
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
        }
    }

    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
        }
    }

    fn from_reqwest(error: reqwest::Error) -> Self {
        Self {
            transient: error.is_timeout() || error.is_connect() || error.is_request(),
            message: error.to_string(),
        }
    }
}

impl Display for VeoHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Copy)]
enum ErrorContext {
    Submit,
    Poll,
    Download,
}

impl ErrorContext {
    fn error(self, message: String) -> VideoProviderError {
        match self {
            Self::Submit => VideoProviderError::SubmitFailed { message },
            Self::Poll => VideoProviderError::PollFailed { message },
            Self::Download => VideoProviderError::DownloadFailed { message },
        }
    }
}

#[derive(Debug, Serialize)]
struct VeoGenerateRequest {
    instances: Vec<VeoGenerateInstance>,
    parameters: VeoGenerateParameters,
}

impl From<VideoGenerationRequest> for VeoGenerateRequest {
    fn from(request: VideoGenerationRequest) -> Self {
        Self {
            instances: vec![VeoGenerateInstance {
                prompt: request.prompt.text,
            }],
            parameters: VeoGenerateParameters {
                aspect_ratio: request.prompt.aspect_ratio,
                duration_seconds: request.prompt.duration_seconds,
                negative_prompt: request.prompt.negative_prompt,
                number_of_videos: 1,
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct VeoGenerateInstance {
    prompt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VeoGenerateParameters {
    aspect_ratio: String,
    duration_seconds: u16,
    negative_prompt: String,
    number_of_videos: u8,
}

#[derive(Debug, Deserialize)]
struct VeoOperation {
    name: String,
    done: Option<bool>,
    error: Option<VeoOperationError>,
    response: Option<VeoOperationResponse>,
}

impl VeoOperation {
    fn video_uri(&self) -> Option<&str> {
        self.response
            .as_ref()?
            .generate_video_response
            .generated_samples
            .first()?
            .video
            .uri
            .as_deref()
    }
}

#[derive(Debug, Deserialize)]
struct VeoOperationError {
    code: Option<i32>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VeoOperationResponse {
    generate_video_response: VeoGenerateVideoResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VeoGenerateVideoResponse {
    generated_samples: Vec<VeoGeneratedSample>,
}

#[derive(Debug, Deserialize)]
struct VeoGeneratedSample {
    video: VeoGeneratedVideo,
}

#[derive(Debug, Deserialize)]
struct VeoGeneratedVideo {
    uri: Option<String>,
}

fn trim_trailing_slashes(value: String) -> String {
    value.trim_end_matches('/').to_owned()
}

fn file_extension_for_content_type(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ => "bin",
    }
}
