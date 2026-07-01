use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use dave_wang_6c2c_daily_video::{
    models::rotation::RotationAnimal,
    providers::video::{
        VideoGenerationRequest, VideoJob, VideoJobStatus, VideoProvider, VideoProviderError,
        VideoProviderKind, build_funny_video_prompt,
        veo::{
            HttpMethod, HttpRequest, HttpResponse, RetryPolicy, VeoHttpClient, VeoHttpError,
            VeoVideoProvider,
        },
    },
};
use serde_json::json;

#[tokio::test]
async fn submit_prompt_posts_predict_long_running_and_retries_transient_http() {
    let http = Arc::new(MockHttpClient::new(vec![
        Ok(HttpResponse::bytes(503, "text/plain", b"busy".to_vec())),
        HttpResponse::json(200, json!({ "name": "operations/video-123" }))
            .map(Ok)
            .unwrap(),
    ]));
    let provider = test_provider(http.clone());
    let prompt = build_funny_video_prompt(RotationAnimal::Cat);

    let job = provider
        .submit_prompt(VideoGenerationRequest::new(prompt))
        .await
        .expect("submit should succeed after retry");

    assert_eq!(job.provider, VideoProviderKind::GeminiVeo);
    assert_eq!(job.provider_job_id, "operations/video-123");

    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(
        requests[0].url,
        "https://gemini.test/v1beta/models/veo-3.1-generate-preview:predictLongRunning"
    );
    assert_header(&requests[0], "x-goog-api-key", "test-key");
    assert_header(&requests[0], "content-type", "application/json");

    let body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().expect("request body")).unwrap();
    assert_eq!(
        body["instances"][0]["prompt"]
            .as_str()
            .expect("prompt string")
            .contains("cat"),
        true
    );
    assert_eq!(body["parameters"]["aspectRatio"], "9:16");
    assert_eq!(body["parameters"]["durationSeconds"], 8);
    assert_eq!(body["parameters"]["numberOfVideos"], 1);
    assert!(
        body["parameters"]["negativePrompt"]
            .as_str()
            .expect("negative prompt")
            .contains("watermarks")
    );
}

#[tokio::test]
async fn submit_prompt_fails_when_operation_name_is_missing() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(200, json!({ "done": false }))
            .map(Ok)
            .unwrap(),
    ]));
    let provider = test_provider(http);
    let prompt = build_funny_video_prompt(RotationAnimal::Dog);

    let error = provider
        .submit_prompt(VideoGenerationRequest::new(prompt))
        .await
        .expect_err("missing operation name should fail");

    assert!(matches!(error, VideoProviderError::SubmitFailed { .. }));
}

#[tokio::test]
async fn poll_job_maps_running_complete_and_failed_statuses() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(200, json!({ "name": "operations/video-123", "done": false }))
            .map(Ok)
            .unwrap(),
        HttpResponse::json(
            200,
            json!({
                "name": "operations/video-123",
                "done": true,
                "response": {
                    "generateVideoResponse": {
                        "generatedSamples": [{ "video": { "uri": "https://download.test/video.mp4" } }]
                    }
                }
            }),
        )
        .map(Ok)
        .unwrap(),
        HttpResponse::json(
            200,
            json!({
                "name": "operations/video-123",
                "done": true,
                "error": { "code": 13, "message": "generation failed" }
            }),
        )
        .map(Ok)
        .unwrap(),
    ]));
    let provider = test_provider(http.clone());
    let job = test_job();

    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        VideoJobStatus::Running
    );
    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        VideoJobStatus::Complete
    );
    assert_eq!(
        provider.poll_job(&job).await.unwrap(),
        VideoJobStatus::Failed {
            message: "generation failed".to_owned()
        }
    );

    let requests = http.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.url == "https://gemini.test/v1beta/operations/video-123")
    );
}

#[tokio::test]
async fn download_clip_reads_operation_uri_then_downloads_bytes() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(
            200,
            json!({
                "name": "operations/video-123",
                "done": true,
                "response": {
                    "generateVideoResponse": {
                        "generatedSamples": [{ "video": { "uri": "https://download.test/video.mp4" } }]
                    }
                }
            }),
        )
        .map(Ok)
        .unwrap(),
        Ok(HttpResponse::bytes(200, "video/mp4", b"mp4-bytes".to_vec())),
    ]));
    let provider = test_provider(http.clone());

    let clip = provider
        .download_clip(&test_job())
        .await
        .expect("download should succeed");

    assert_eq!(clip.bytes, b"mp4-bytes");
    assert_eq!(clip.content_type, "video/mp4");
    assert_eq!(clip.file_extension, "mp4");

    let requests = http.requests();
    assert_eq!(
        requests[0].url,
        "https://gemini.test/v1beta/operations/video-123"
    );
    assert_eq!(requests[1].url, "https://download.test/video.mp4");
    assert_header(&requests[1], "x-goog-api-key", "test-key");
}

#[tokio::test]
async fn download_clip_retries_transient_network_errors() {
    let http = Arc::new(MockHttpClient::new(vec![
        HttpResponse::json(
            200,
            json!({
                "name": "operations/video-123",
                "done": true,
                "response": {
                    "generateVideoResponse": {
                        "generatedSamples": [{ "video": { "uri": "https://download.test/video.mp4" } }]
                    }
                }
            }),
        )
        .map(Ok)
        .unwrap(),
        Err(VeoHttpError::transient("connection reset")),
        Ok(HttpResponse::bytes(200, "video/webm", b"webm-bytes".to_vec())),
    ]));
    let provider = test_provider(http.clone());

    let clip = provider
        .download_clip(&test_job())
        .await
        .expect("download should retry transient error");

    assert_eq!(clip.bytes, b"webm-bytes");
    assert_eq!(clip.file_extension, "webm");
    assert_eq!(http.requests().len(), 3);
}

fn test_provider(http: Arc<MockHttpClient>) -> VeoVideoProvider {
    VeoVideoProvider::with_options(
        "test-key",
        "veo-3.1-generate-preview",
        "https://gemini.test/v1beta",
        http,
        RetryPolicy::no_delay(3),
    )
}

fn test_job() -> VideoJob {
    VideoJob {
        provider: VideoProviderKind::GeminiVeo,
        provider_job_id: "operations/video-123".to_owned(),
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
    responses: Mutex<VecDeque<Result<HttpResponse, VeoHttpError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl MockHttpClient {
    fn new(responses: Vec<Result<HttpResponse, VeoHttpError>>) -> Self {
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
impl VeoHttpClient for MockHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, VeoHttpError> {
        self.requests.lock().expect("requests lock").push(request);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .expect("mock response")
    }
}
