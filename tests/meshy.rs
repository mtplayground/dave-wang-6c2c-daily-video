use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use dave_wang_6c2c_daily_video::{
    config::ProviderConfig,
    providers::image_to_3d::{
        ImageTo3DJob, ImageTo3DJobStatus, ImageTo3DProvider, ImageTo3DProviderError,
        ImageTo3DProviderKind, ImageTo3DRequest,
        meshy::{
            HttpMethod, HttpRequest, HttpResponse, MeshyHttpClient, MeshyHttpError,
            MeshyImageTo3DProvider, RetryPolicy,
        },
    },
};
use serde_json::json;

#[tokio::test]
async fn submit_image_posts_textured_glb_request_and_retries_transient_http() {
    let http = Arc::new(MockHttpClient::new(vec![
        Ok(HttpResponse::bytes(
            429,
            "text/plain",
            b"rate limit".to_vec(),
        )),
        HttpResponse::json(200, json!({ "result": "task-123" }))
            .map(Ok)
            .unwrap(),
    ]));
    let provider = test_provider(http.clone());
    let request = ImageTo3DRequest::new("https://assets.test/frame.png")
        .with_texture_prompt("preserve the animal's playful fur and color");

    let job = provider
        .submit_image(request)
        .await
        .expect("submit should succeed after retry");

    assert_eq!(job.provider, ImageTo3DProviderKind::Meshy);
    assert_eq!(job.provider_job_id, "task-123");

    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(requests[0].url, "https://meshy.test/openapi/v1/image-to-3d");
    assert_header(&requests[0], "authorization", "Bearer test-key");
    assert_header(&requests[0], "content-type", "application/json");

    let body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().expect("request body")).unwrap();
    assert_eq!(body["image_url"], "https://assets.test/frame.png");
    assert_eq!(body["ai_model"], "latest");
    assert_eq!(body["model_type"], "standard");
    assert_eq!(body["should_texture"], true);
    assert_eq!(body["enable_pbr"], true);
    assert_eq!(body["should_remesh"], true);
    assert_eq!(body["target_polycount"], 100_000);
    assert_eq!(body["target_formats"], json!(["glb"]));
    assert_eq!(
        body["texture_prompt"],
        "preserve the animal's playful fur and color"
    );
}

#[tokio::test]
async fn submit_image_fails_when_task_id_is_missing() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(200, json!({ "result": "" }))
            .map(Ok)
            .unwrap(),
    ]));
    let provider = test_provider(http);

    let error = provider
        .submit_image(ImageTo3DRequest::new("https://assets.test/frame.png"))
        .await
        .expect_err("missing result should fail");

    assert!(matches!(error, ImageTo3DProviderError::SubmitFailed { .. }));
}

#[tokio::test]
async fn poll_job_maps_pending_running_complete_and_failed_statuses() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(
            200,
            json!({ "id": "task-123", "status": "PENDING", "progress": 0 }),
        )
        .map(Ok)
        .unwrap(),
        HttpResponse::json(
            200,
            json!({ "id": "task-123", "status": "IN_PROGRESS", "progress": 43 }),
        )
        .map(Ok)
        .unwrap(),
        HttpResponse::json(
            200,
            json!({
                "id": "task-123",
                "status": "SUCCEEDED",
                "progress": 100,
                "model_urls": { "glb": "https://assets.meshy.test/model.glb" }
            }),
        )
        .map(Ok)
        .unwrap(),
        HttpResponse::json(
            200,
            json!({
                "id": "task-123",
                "status": "FAILED",
                "task_error": { "message": "input image rejected" }
            }),
        )
        .map(Ok)
        .unwrap(),
    ]));
    let provider = test_provider(http.clone());
    let job = test_job();

    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        ImageTo3DJobStatus::Pending
    );
    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        ImageTo3DJobStatus::Running { progress: Some(43) }
    );
    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        ImageTo3DJobStatus::Complete
    );
    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        ImageTo3DJobStatus::Failed {
            message: "input image rejected".to_owned()
        }
    );

    let requests = http.requests();
    assert_eq!(requests.len(), 4);
    assert!(
        requests
            .iter()
            .all(|request| request.url == "https://meshy.test/openapi/v1/image-to-3d/task-123")
    );
}

#[tokio::test]
async fn download_glb_reads_task_then_downloads_signed_model_url_without_bearer_auth() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(
            200,
            json!({
                "id": "task-123",
                "status": "SUCCEEDED",
                "progress": 100,
                "model_urls": { "glb": "https://assets.meshy.test/model.glb?Expires=123" }
            }),
        )
        .map(Ok)
        .unwrap(),
        Ok(HttpResponse::bytes(
            200,
            "model/gltf-binary",
            b"glb-bytes".to_vec(),
        )),
    ]));
    let provider = test_provider(http.clone());

    let model = provider
        .download_glb(&test_job())
        .await
        .expect("download should succeed");

    assert_eq!(model.bytes, b"glb-bytes");
    assert_eq!(model.content_type, "model/gltf-binary");
    assert_eq!(model.file_extension, "glb");

    let requests = http.requests();
    assert_eq!(
        requests[0].url,
        "https://meshy.test/openapi/v1/image-to-3d/task-123"
    );
    assert_header(&requests[0], "authorization", "Bearer test-key");
    assert_eq!(
        requests[1].url,
        "https://assets.meshy.test/model.glb?Expires=123"
    );
    assert_eq!(
        requests[1]
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization")),
        None
    );
}

#[tokio::test]
async fn download_glb_retries_transient_asset_download_errors() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(
            200,
            json!({
                "id": "task-123",
                "status": "SUCCEEDED",
                "model_urls": { "glb": "https://assets.meshy.test/model.glb" }
            }),
        )
        .map(Ok)
        .unwrap(),
        Err(MeshyHttpError::transient("asset timeout")),
        Ok(HttpResponse::bytes(
            200,
            "application/octet-stream",
            b"glb-retry".to_vec(),
        )),
    ]));
    let provider = test_provider(http.clone());

    let model = provider
        .download_glb(&test_job())
        .await
        .expect("asset download should retry transient error");

    assert_eq!(model.bytes, b"glb-retry");
    assert_eq!(model.file_extension, "glb");
    assert_eq!(http.requests().len(), 3);
}

#[test]
fn provider_kind_is_selected_from_config() {
    let config = ProviderConfig {
        video_provider: "gemini_veo".to_owned(),
        image_to_3d_provider: "meshy".to_owned(),
        gemini_api_key: "gemini-key".to_owned(),
        meshy_api_key: "meshy-key".to_owned(),
    };

    assert_eq!(
        ImageTo3DProviderKind::from_config(&config),
        Ok(ImageTo3DProviderKind::Meshy)
    );
    assert!(MeshyImageTo3DProvider::from_config(&config).is_ok());
}

fn test_provider(http: Arc<MockHttpClient>) -> MeshyImageTo3DProvider {
    MeshyImageTo3DProvider::with_options(
        "test-key",
        "https://meshy.test/openapi/v1",
        http,
        RetryPolicy::no_delay(3),
    )
}

fn test_job() -> ImageTo3DJob {
    ImageTo3DJob {
        provider: ImageTo3DProviderKind::Meshy,
        provider_job_id: "task-123".to_owned(),
    }
}

fn assert_header(request: &HttpRequest, name: &str, expected: &str) {
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str()),
        Some(expected)
    );
}

struct MockHttpClient {
    responses: Mutex<VecDeque<Result<HttpResponse, MeshyHttpError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl MockHttpClient {
    fn new(responses: Vec<Result<HttpResponse, MeshyHttpError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl MeshyHttpClient for MockHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, MeshyHttpError> {
        self.requests.lock().expect("requests lock").push(request);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .expect("mock response")
    }
}
