#![allow(dead_code)]

use std::{
    fmt::{self, Display},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{error, warn};

use crate::error::provider_http_category;
use super::{
    GlbModel, ImageTo3DJob, ImageTo3DJobStatus, ImageTo3DProvider, ImageTo3DProviderError,
    ImageTo3DProviderKind, ImageTo3DRequest,
};
use crate::config::ProviderConfig;

pub const DEFAULT_MESHY_BASE_URL: &str = "https://api.meshy.ai/openapi/v1";

#[derive(Clone)]
pub struct MeshyImageTo3DProvider {
    api_key: String,
    base_url: String,
    http: Arc<dyn MeshyHttpClient>,
    retry_policy: RetryPolicy,
}

impl MeshyImageTo3DProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_MESHY_BASE_URL.to_owned(),
            http: Arc::new(ReqwestMeshyHttpClient::default()),
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn from_config(config: &ProviderConfig) -> Result<Self, ImageTo3DProviderError> {
        ImageTo3DProviderKind::from_config(config)?;
        match config.meshy_api_key.clone() {
            Some(api_key) => Ok(Self::new(api_key)),
            None => Err(ImageTo3DProviderError::InvalidProvider {
                name: "meshy requires MESHY_API_KEY".to_owned(),
            }),
        }
    }

    pub fn with_options(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        http: Arc<dyn MeshyHttpClient>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: trim_trailing_slashes(base_url.into()),
            http,
            retry_policy,
        }
    }

    pub async fn wait_until_ready(
        &self,
        job: &ImageTo3DJob,
    ) -> Result<ImageTo3DJobStatus, ImageTo3DProviderError> {
        loop {
            match self.poll_job(job).await? {
                ImageTo3DJobStatus::Pending | ImageTo3DJobStatus::Running { .. } => {
                    sleep(self.retry_policy.poll_interval).await;
                }
                terminal => return Ok(terminal),
            }
        }
    }

    fn task_collection_url(&self) -> String {
        format!("{}/image-to-3d", self.base_url)
    }

    fn task_url(&self, task_id: &str) -> String {
        format!(
            "{}/image-to-3d/{}",
            self.base_url,
            task_id.trim_matches('/')
        )
    }

    async fn send_with_retries(
        &self,
        request: HttpRequest,
        context: ErrorContext,
    ) -> Result<HttpResponse, ImageTo3DProviderError> {
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
                    warn!(
                        provider = "meshy",
                        context = %context.as_str(),
                        attempt = attempts,
                        status = response.status,
                        category = %provider_http_category(response.status),
                        "transient image-to-3d provider HTTP response; retrying"
                    );
                    sleep(delay).await;
                    delay = self.retry_policy.next_delay(delay);
                }
                Ok(response) => {
                    let category = provider_http_category(response.status);
                    error!(
                        provider = "meshy",
                        context = %context.as_str(),
                        attempts,
                        status = response.status,
                        category = %category,
                        "image-to-3d provider HTTP request failed"
                    );
                    return Err(context.error(format!(
                        "[{category}] Meshy returned HTTP {}: {}",
                        response.status,
                        response.text_lossy()
                    )));
                }
                Err(error) if error.transient && attempts < self.retry_policy.max_attempts => {
                    warn!(
                        provider = "meshy",
                        context = %context.as_str(),
                        attempt = attempts,
                        error = %error,
                        category = "provider_transient",
                        "transient image-to-3d provider transport error; retrying"
                    );
                    sleep(delay).await;
                    delay = self.retry_policy.next_delay(delay);
                }
                Err(error) => {
                    error!(
                        provider = "meshy",
                        context = %context.as_str(),
                        attempts,
                        error = %error,
                        category = "provider_transient",
                        "image-to-3d provider transport request failed"
                    );
                    return Err(context.error(format!("[provider_transient] {error}")));
                }
            }
        }
    }

    async fn get_task(&self, task_id: &str) -> Result<MeshyTask, ImageTo3DProviderError> {
        let response = self
            .send_with_retries(
                HttpRequest::get(self.task_url(task_id)).with_bearer_auth(&self.api_key),
                ErrorContext::Poll,
            )
            .await?;

        response.decode_json(ErrorContext::Poll)
    }

    async fn download_url(&self, url: &str) -> Result<GlbModel, ImageTo3DProviderError> {
        let response = self
            .send_with_retries(HttpRequest::get(url.to_owned()), ErrorContext::Download)
            .await?;
        let content_type = response
            .header("content-type")
            .unwrap_or("model/gltf-binary")
            .to_owned();

        Ok(GlbModel {
            bytes: response.body,
            content_type,
            file_extension: "glb".to_owned(),
        })
    }
}

#[async_trait]
impl ImageTo3DProvider for MeshyImageTo3DProvider {
    async fn submit_image(
        &self,
        request: ImageTo3DRequest,
    ) -> Result<ImageTo3DJob, ImageTo3DProviderError> {
        let body = serde_json::to_vec(&MeshyCreateTaskRequest::from(request)).map_err(|error| {
            ImageTo3DProviderError::SubmitFailed {
                message: format!("failed to encode request JSON: {error}"),
            }
        })?;

        let response = self
            .send_with_retries(
                HttpRequest::post_json(self.task_collection_url(), body)
                    .with_bearer_auth(&self.api_key),
                ErrorContext::Submit,
            )
            .await?;
        let create_response: MeshyCreateTaskResponse =
            response.decode_json(ErrorContext::Submit)?;

        if create_response.result.trim().is_empty() {
            return Err(ImageTo3DProviderError::SubmitFailed {
                message: "Meshy response did not include a task id".to_owned(),
            });
        }

        Ok(ImageTo3DJob {
            provider: ImageTo3DProviderKind::Meshy,
            provider_job_id: create_response.result,
        })
    }

    async fn poll_job(
        &self,
        job: &ImageTo3DJob,
    ) -> Result<ImageTo3DJobStatus, ImageTo3DProviderError> {
        if job.provider != ImageTo3DProviderKind::Meshy {
            return Err(ImageTo3DProviderError::PollFailed {
                message: format!("cannot poll {} job with Meshy provider", job.provider),
            });
        }

        let task = self.get_task(&job.provider_job_id).await?;
        Ok(task_status(&task))
    }

    async fn download_glb(&self, job: &ImageTo3DJob) -> Result<GlbModel, ImageTo3DProviderError> {
        if job.provider != ImageTo3DProviderKind::Meshy {
            return Err(ImageTo3DProviderError::DownloadFailed {
                message: format!("cannot download {} job with Meshy provider", job.provider),
            });
        }

        let task = self.get_task(&job.provider_job_id).await?;
        match task_status(&task) {
            ImageTo3DJobStatus::Complete => {
                let glb_url = task.model_urls.and_then(|urls| urls.glb).ok_or_else(|| {
                    ImageTo3DProviderError::DownloadFailed {
                        message: "Meshy task completed without a GLB URL".to_owned(),
                    }
                })?;

                self.download_url(&glb_url).await
            }
            ImageTo3DJobStatus::Failed { message } => {
                Err(ImageTo3DProviderError::DownloadFailed { message })
            }
            ImageTo3DJobStatus::Pending | ImageTo3DJobStatus::Running { .. } => {
                Err(ImageTo3DProviderError::DownloadFailed {
                    message: "Meshy task is not complete".to_owned(),
                })
            }
        }
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
pub trait MeshyHttpClient: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, MeshyHttpError>;
}

#[derive(Default)]
struct ReqwestMeshyHttpClient {
    client: reqwest::Client,
}

#[async_trait]
impl MeshyHttpClient for ReqwestMeshyHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, MeshyHttpError> {
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

        let response = builder.send().await.map_err(MeshyHttpError::from_reqwest)?;
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
            .map_err(MeshyHttpError::from_reqwest)?
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

    pub fn with_bearer_auth(mut self, api_key: &str) -> Self {
        self.headers
            .push(("authorization".to_owned(), format!("Bearer {api_key}")));
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
    ) -> Result<T, ImageTo3DProviderError> {
        serde_json::from_slice(&self.body).map_err(|error| {
            context.error(format!("failed to decode Meshy JSON response: {error}"))
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
pub struct MeshyHttpError {
    pub message: String,
    pub transient: bool,
}

impl MeshyHttpError {
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

impl Display for MeshyHttpError {
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
    fn as_str(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Poll => "poll",
            Self::Download => "download",
        }
    }

    fn error(self, message: String) -> ImageTo3DProviderError {
        match self {
            Self::Submit => ImageTo3DProviderError::SubmitFailed { message },
            Self::Poll => ImageTo3DProviderError::PollFailed { message },
            Self::Download => ImageTo3DProviderError::DownloadFailed { message },
        }
    }
}

#[derive(Debug, Serialize)]
struct MeshyCreateTaskRequest {
    image_url: String,
    ai_model: String,
    model_type: String,
    should_texture: bool,
    enable_pbr: bool,
    should_remesh: bool,
    target_polycount: u32,
    target_formats: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    texture_prompt: Option<String>,
}

impl From<ImageTo3DRequest> for MeshyCreateTaskRequest {
    fn from(request: ImageTo3DRequest) -> Self {
        Self {
            image_url: request.image_url,
            ai_model: request.ai_model,
            model_type: request.model_type.as_str().to_owned(),
            should_texture: request.should_texture,
            enable_pbr: request.enable_pbr,
            should_remesh: request.should_remesh,
            target_polycount: request.target_polycount,
            target_formats: request.target_formats,
            texture_prompt: request.texture_prompt,
        }
    }
}

#[derive(Debug, Deserialize)]
struct MeshyCreateTaskResponse {
    result: String,
}

#[derive(Debug, Deserialize)]
struct MeshyTask {
    status: String,
    progress: Option<u8>,
    model_urls: Option<MeshyModelUrls>,
    task_error: Option<MeshyTaskError>,
}

#[derive(Debug, Deserialize)]
struct MeshyModelUrls {
    glb: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MeshyTaskError {
    message: Option<String>,
}

fn task_status(task: &MeshyTask) -> ImageTo3DJobStatus {
    match task.status.as_str() {
        "PENDING" => ImageTo3DJobStatus::Pending,
        "IN_PROGRESS" => ImageTo3DJobStatus::Running {
            progress: task.progress,
        },
        "SUCCEEDED" => ImageTo3DJobStatus::Complete,
        "FAILED" | "CANCELED" | "EXPIRED" => ImageTo3DJobStatus::Failed {
            message: task
                .task_error
                .as_ref()
                .and_then(|error| error.message.as_ref())
                .filter(|message| !message.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Meshy task ended with status {}", task.status)),
        },
        unknown => ImageTo3DJobStatus::Failed {
            message: format!("Meshy task returned unknown status {unknown}"),
        },
    }
}

fn trim_trailing_slashes(value: String) -> String {
    value.trim_end_matches('/').to_owned()
}
