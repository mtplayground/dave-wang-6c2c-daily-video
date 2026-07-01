use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use dave_wang_6c2c_daily_video::{
    models::{
        artifact::ArtifactType,
        rotation::{ROTATION_STATE_KEY, RotationAnimal, RotationState},
        run::{PipelineStep, PipelineStepStatus, Run, RunStatus},
        video::PublishedVideo,
    },
    pipeline::{
        ArtifactStorage, Pipeline, PipelineError, PipelineMedia, PipelinePollOptions,
        RunRepository,
    },
    providers::{
        image_to_3d::{
            GlbModel, ImageTo3DJob, ImageTo3DJobStatus, ImageTo3DProvider, ImageTo3DProviderError,
            ImageTo3DProviderKind, ImageTo3DRequest,
        },
        video::{
            VideoClip, VideoGenerationRequest, VideoJob, VideoJobStatus, VideoProvider,
            VideoProviderError, VideoProviderKind,
        },
    },
    repo::{NewArtifact, NewPublishedVideo, RepoError, RunStatusUpdate},
    storage::{ArtifactKind, StorageError, StoredObject},
};
use uuid::Uuid;

#[tokio::test]
async fn full_pipeline_produces_uploads_and_feeds_final_video() -> Result<(), Box<dyn Error>> {
    let repo = Arc::new(MockRepository::new(RotationAnimal::Dog));
    let storage = MockStorage::default();
    let workspace = test_workspace();
    let date = NaiveDate::from_ymd_opt(2099, 12, 20).expect("valid e2e date");
    let pipeline = Pipeline::with_components(
        repo.clone(),
        Arc::new(storage.clone()),
        Arc::new(MockMedia),
        Arc::new(MockVideoProvider),
        Arc::new(MockImageTo3DProvider),
        workspace.clone(),
    )
    .with_poll_options(PipelinePollOptions {
        max_attempts: 1,
        interval: Duration::from_millis(1),
    });

    let outcome = pipeline.start_daily_run(date).await?;

    assert_eq!(outcome.run.date, date);
    assert_eq!(outcome.run.status, RunStatus::Complete);
    assert_eq!(outcome.run.current_step, None);
    assert_eq!(repo.current_animal(), RotationAnimal::Cat);
    assert_eq!(repo.completed_step_count(outcome.run.id), PipelineStep::ORDERED.len());

    let final_mp4 = workspace.join(outcome.run.id.to_string()).join("final.mp4");
    assert_eq!(fs::read(final_mp4)?, b"final-mp4");

    let final_upload = storage
        .uploaded_file("final.mp4")
        .expect("final MP4 should be uploaded");
    assert_eq!(final_upload.bytes, b"final-mp4");
    assert_eq!(final_upload.content_type.as_deref(), Some("video/mp4"));
    assert!(final_upload.storage_key.starts_with("test-prefix/"));

    let feed = repo.feed();
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].run_id, outcome.run.id);
    assert_eq!(feed[0].date, date);
    assert_eq!(feed[0].animal, "cat");
    assert_eq!(feed[0].title, "Daily cat 3D print reveal");
    assert_eq!(feed[0].final_video_storage_key, final_upload.storage_key);
    assert_eq!(outcome.published_video, Some(feed[0].clone()));

    let _ = fs::remove_dir_all(workspace);
    Ok(())
}

fn test_workspace() -> PathBuf {
    std::env::temp_dir().join(format!("daily-video-e2e-{}", Uuid::new_v4()))
}

#[derive(Clone)]
struct MockRepository {
    inner: Arc<Mutex<MockRepositoryState>>,
}

impl MockRepository {
    fn new(current_animal: RotationAnimal) -> Self {
        Self {
            inner: Arc::new(Mutex::new(MockRepositoryState {
                current_animal,
                runs: HashMap::new(),
                step_states: Vec::new(),
                artifacts: Vec::new(),
                published_videos: Vec::new(),
            })),
        }
    }

    fn current_animal(&self) -> RotationAnimal {
        self.inner.lock().expect("repo lock").current_animal
    }

    fn completed_step_count(&self, run_id: Uuid) -> usize {
        self.inner
            .lock()
            .expect("repo lock")
            .step_states
            .iter()
            .filter(|((id, _), status)| *id == run_id && *status == PipelineStepStatus::Complete)
            .count()
    }

    fn feed(&self) -> Vec<PublishedVideo> {
        let mut videos = self
            .inner
            .lock()
            .expect("repo lock")
            .published_videos
            .clone();
        videos.sort_by(|left, right| right.date.cmp(&left.date));
        videos
    }
}

struct MockRepositoryState {
    current_animal: RotationAnimal,
    runs: HashMap<Uuid, Run>,
    step_states: Vec<((Uuid, PipelineStep), PipelineStepStatus)>,
    artifacts: Vec<RecordedArtifact>,
    published_videos: Vec<PublishedVideo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedArtifact {
    run_id: Uuid,
    artifact_type: ArtifactType,
    storage_key: String,
}

#[async_trait]
impl RunRepository for MockRepository {
    async fn advance_rotation(&self) -> Result<RotationState, RepoError> {
        let mut inner = self.inner.lock().expect("repo lock");
        inner.current_animal = inner.current_animal.next();
        Ok(rotation_state(inner.current_animal))
    }

    async fn create_run(&self, date: NaiveDate, animal: &str) -> Result<Run, RepoError> {
        let run = Run {
            id: Uuid::new_v4(),
            date,
            animal: animal.to_owned(),
            status: RunStatus::Pending,
            current_step: None,
            error: None,
            created_at: epoch(),
            updated_at: epoch(),
        };
        self.inner
            .lock()
            .expect("repo lock")
            .runs
            .insert(run.id, run.clone());
        Ok(run)
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<Run>, RepoError> {
        Ok(self.inner.lock().expect("repo lock").runs.get(&id).cloned())
    }

    async fn update_run_status(
        &self,
        id: Uuid,
        _current_status: RunStatus,
        update: RunStatusUpdate,
        error: Option<&str>,
    ) -> Result<Run, RepoError> {
        let mut inner = self.inner.lock().expect("repo lock");
        let run = inner.runs.get_mut(&id).expect("run exists");
        run.status = update.status;
        run.current_step = update.current_step;
        run.error = error.map(str::to_owned);
        run.updated_at = epoch();
        Ok(run.clone())
    }

    async fn upsert_step_state(
        &self,
        run_id: Uuid,
        step: PipelineStep,
        status: PipelineStepStatus,
        _error: Option<&str>,
    ) -> Result<(), RepoError> {
        let mut inner = self.inner.lock().expect("repo lock");
        if let Some((_, existing_status)) = inner
            .step_states
            .iter_mut()
            .find(|((id, candidate), _)| *id == run_id && *candidate == step)
        {
            *existing_status = status;
        } else {
            inner.step_states.push(((run_id, step), status));
        }
        Ok(())
    }

    async fn record_artifact(&self, artifact: NewArtifact<'_>) -> Result<(), RepoError> {
        self.inner
            .lock()
            .expect("repo lock")
            .artifacts
            .push(RecordedArtifact {
                run_id: artifact.run_id,
                artifact_type: artifact.artifact_type,
                storage_key: artifact.storage_key.to_owned(),
            });
        Ok(())
    }

    async fn record_published_video(
        &self,
        video: NewPublishedVideo<'_>,
    ) -> Result<PublishedVideo, RepoError> {
        let published = PublishedVideo {
            id: Uuid::new_v4(),
            run_id: video.run_id,
            date: video.date,
            animal: video.animal.to_owned(),
            title: video.title.to_owned(),
            final_video_storage_key: video.final_video_storage_key.to_owned(),
            published_at: epoch(),
            created_at: epoch(),
            updated_at: epoch(),
        };
        self.inner
            .lock()
            .expect("repo lock")
            .published_videos
            .push(published.clone());
        Ok(published)
    }
}

fn rotation_state(current_animal: RotationAnimal) -> RotationState {
    RotationState {
        key: ROTATION_STATE_KEY.to_owned(),
        current_position: current_animal.position(),
        current_animal,
        created_at: epoch(),
        updated_at: epoch(),
    }
}

fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

#[derive(Clone, Default)]
struct MockStorage {
    uploads: Arc<Mutex<Vec<Upload>>>,
}

impl MockStorage {
    fn uploaded_file(&self, file_name: &str) -> Option<Upload> {
        self.uploads
            .lock()
            .expect("uploads lock")
            .iter()
            .find(|upload| upload.relative_key.ends_with(file_name))
            .cloned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Upload {
    relative_key: String,
    storage_key: String,
    bytes: Vec<u8>,
    content_type: Option<String>,
}

#[async_trait]
impl ArtifactStorage for MockStorage {
    async fn upload_artifact(
        &self,
        relative_key: String,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<StoredObject, StorageError> {
        let stored = StoredObject {
            storage_key: format!("test-prefix/{relative_key}"),
            relative_key,
        };
        self.uploads.lock().expect("uploads lock").push(Upload {
            relative_key: stored.relative_key.clone(),
            storage_key: stored.storage_key.clone(),
            bytes,
            content_type: content_type.map(str::to_owned),
        });
        Ok(stored)
    }

    async fn public_url(
        &self,
        relative_key: &str,
        _expires_in: Duration,
    ) -> Result<String, StorageError> {
        Ok(format!("https://storage.test/{relative_key}"))
    }

    fn artifact_key(
        &self,
        kind: ArtifactKind,
        run_id: Uuid,
        file_name: &str,
    ) -> Result<String, StorageError> {
        let folder = match kind {
            ArtifactKind::RawVideo => "raw-videos",
            ArtifactKind::Frame => "frames",
            ArtifactKind::Glb => "glb",
            ArtifactKind::RevealClip => "reveal-clips",
            ArtifactKind::FinalMp4 => "final-videos",
        };
        Ok(format!("artifacts/{run_id}/{folder}/{file_name}"))
    }
}

struct MockMedia;

#[async_trait]
impl PipelineMedia for MockMedia {
    async fn extract_frame(&self, _input: &Path, output: &Path) -> Result<String, PipelineError> {
        fs::write(output, b"frame")?;
        Ok("image/jpeg".to_owned())
    }

    async fn render_reveal(&self, _input: &Path, output: &Path) -> Result<String, PipelineError> {
        fs::write(output, b"reveal")?;
        Ok("video/mp4".to_owned())
    }

    async fn assemble_final_video(
        &self,
        _funny_video: &Path,
        _reveal_clip: &Path,
        output: &Path,
    ) -> Result<(), PipelineError> {
        fs::write(output, b"final-mp4")?;
        Ok(())
    }
}

struct MockVideoProvider;

#[async_trait]
impl VideoProvider for MockVideoProvider {
    async fn submit_prompt(
        &self,
        _request: VideoGenerationRequest,
    ) -> Result<VideoJob, VideoProviderError> {
        Ok(VideoJob {
            provider: VideoProviderKind::GeminiVeo,
            provider_job_id: "e2e-video-job".to_owned(),
        })
    }

    async fn poll_job(&self, _job: &VideoJob) -> Result<VideoJobStatus, VideoProviderError> {
        Ok(VideoJobStatus::Complete)
    }

    async fn download_clip(&self, _job: &VideoJob) -> Result<VideoClip, VideoProviderError> {
        Ok(VideoClip {
            bytes: b"raw-video".to_vec(),
            content_type: "video/mp4".to_owned(),
            file_extension: "mp4".to_owned(),
        })
    }
}

struct MockImageTo3DProvider;

#[async_trait]
impl ImageTo3DProvider for MockImageTo3DProvider {
    async fn submit_image(
        &self,
        _request: ImageTo3DRequest,
    ) -> Result<ImageTo3DJob, ImageTo3DProviderError> {
        Ok(ImageTo3DJob {
            provider: ImageTo3DProviderKind::Meshy,
            provider_job_id: "e2e-mesh-job".to_owned(),
        })
    }

    async fn poll_job(
        &self,
        _job: &ImageTo3DJob,
    ) -> Result<ImageTo3DJobStatus, ImageTo3DProviderError> {
        Ok(ImageTo3DJobStatus::Complete)
    }

    async fn download_glb(&self, _job: &ImageTo3DJob) -> Result<GlbModel, ImageTo3DProviderError> {
        Ok(GlbModel {
            bytes: b"glb".to_vec(),
            content_type: "model/gltf-binary".to_owned(),
            file_extension: "glb".to_owned(),
        })
    }
}
