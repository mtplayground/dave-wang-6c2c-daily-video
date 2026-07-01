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
async fn start_daily_run_sequences_steps_and_advances_rotation() -> Result<(), Box<dyn Error>> {
    let repo = Arc::new(MockRepository::new(RotationAnimal::Dog));
    let events = Events::default();
    let workspace = test_workspace("full");
    let pipeline = test_pipeline(repo.clone(), workspace.clone(), events.clone());
    let date = NaiveDate::from_ymd_opt(2091, 1, 17).expect("valid test date");

    let outcome = pipeline.start_daily_run(date).await?;

    assert_eq!(outcome.run.date, date);
    assert_eq!(outcome.run.status, RunStatus::Complete);
    assert_eq!(outcome.run.current_step, None);
    assert_eq!(repo.current_animal(), RotationAnimal::Cat);
    assert_eq!(
        events.entries(),
        vec![
            "video:submit",
            "video:poll",
            "video:download",
            "storage:upload:raw.mp4",
            "media:extract_frame",
            "storage:upload:frame.jpg",
            "storage:public_url:frame.jpg",
            "image_to_3d:submit",
            "image_to_3d:poll",
            "image_to_3d:download",
            "storage:upload:model.glb",
            "media:render_reveal",
            "storage:upload:reveal.mp4",
            "media:assemble",
            "storage:upload:final.mp4",
        ]
    );
    assert_eq!(repo.completed_step_count(outcome.run.id), PipelineStep::ORDERED.len());
    assert!(repo.published_video_for(outcome.run.id).is_some());

    let _ = fs::remove_dir_all(workspace);
    Ok(())
}

#[tokio::test]
async fn resume_failed_run_starts_at_failed_step() -> Result<(), Box<dyn Error>> {
    let repo = Arc::new(MockRepository::new(RotationAnimal::Dog));
    let date = NaiveDate::from_ymd_opt(2091, 1, 18).expect("valid test date");
    let failed_run = repo.insert_failed_run(date, "dog", PipelineStep::RenderReveal);
    let events = Events::default();
    let workspace = test_workspace("resume");
    let run_dir = workspace.join(failed_run.id.to_string());
    fs::create_dir_all(&run_dir)?;
    fs::write(run_dir.join("raw.mp4"), b"raw-video")?;
    fs::write(run_dir.join("model.glb"), b"glb")?;

    let pipeline = test_pipeline(repo.clone(), workspace.clone(), events.clone());
    let outcome = pipeline.resume_run(failed_run.id).await?;

    assert_eq!(outcome.run.status, RunStatus::Complete);
    assert_eq!(
        events.entries(),
        vec![
            "media:render_reveal",
            "storage:upload:reveal.mp4",
            "media:assemble",
            "storage:upload:final.mp4",
        ]
    );
    assert_eq!(
        repo.step_status(failed_run.id, PipelineStep::RenderReveal),
        Some(PipelineStepStatus::Complete)
    );
    assert_eq!(
        repo.step_status(failed_run.id, PipelineStep::GenerateVideo),
        None
    );

    let _ = fs::remove_dir_all(workspace);
    Ok(())
}

fn test_pipeline(repo: Arc<dyn RunRepository>, workspace: PathBuf, events: Events) -> Pipeline {
    Pipeline::with_components(
        repo,
        Arc::new(MockStorage {
            events: events.clone(),
        }),
        Arc::new(MockMedia {
            events: events.clone(),
        }),
        Arc::new(MockVideoProvider {
            events: events.clone(),
        }),
        Arc::new(MockImageTo3DProvider { events }),
        workspace,
    )
    .with_poll_options(PipelinePollOptions {
        max_attempts: 1,
        interval: Duration::from_millis(1),
    })
}

fn test_workspace(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("daily-video-pipeline-{label}-{}", Uuid::new_v4()))
}

#[derive(Clone, Default)]
struct Events {
    inner: Arc<Mutex<Vec<String>>>,
}

impl Events {
    fn push(&self, event: impl Into<String>) {
        self.inner.lock().expect("events lock").push(event.into());
    }

    fn entries(&self) -> Vec<String> {
        self.inner.lock().expect("events lock").clone()
    }
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
                published_videos: HashMap::new(),
            })),
        }
    }

    fn current_animal(&self) -> RotationAnimal {
        self.inner.lock().expect("repo lock").current_animal
    }

    fn insert_failed_run(&self, date: NaiveDate, animal: &str, step: PipelineStep) -> Run {
        let run = Run {
            id: Uuid::new_v4(),
            date,
            animal: animal.to_owned(),
            status: RunStatus::Failed,
            current_step: Some(step),
            error: Some("forced failure".to_owned()),
            created_at: epoch(),
            updated_at: epoch(),
        };
        let mut inner = self.inner.lock().expect("repo lock");
        inner.set_step_status(run.id, step, PipelineStepStatus::Failed);
        inner.runs.insert(run.id, run.clone());
        run
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

    fn step_status(&self, run_id: Uuid, step: PipelineStep) -> Option<PipelineStepStatus> {
        self.inner
            .lock()
            .expect("repo lock")
            .step_states
            .iter()
            .find_map(|((id, candidate), status)| {
                (*id == run_id && *candidate == step).then_some(*status)
            })
    }

    fn published_video_for(&self, run_id: Uuid) -> Option<PublishedVideo> {
        self.inner
            .lock()
            .expect("repo lock")
            .published_videos
            .get(&run_id)
            .cloned()
    }
}

struct MockRepositoryState {
    current_animal: RotationAnimal,
    runs: HashMap<Uuid, Run>,
    step_states: Vec<((Uuid, PipelineStep), PipelineStepStatus)>,
    published_videos: HashMap<Uuid, PublishedVideo>,
}

impl MockRepositoryState {
    fn set_step_status(&mut self, run_id: Uuid, step: PipelineStep, status: PipelineStepStatus) {
        if let Some((_, existing_status)) = self
            .step_states
            .iter_mut()
            .find(|((id, candidate), _)| *id == run_id && *candidate == step)
        {
            *existing_status = status;
        } else {
            self.step_states.push(((run_id, step), status));
        }
    }
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
        self.inner
            .lock()
            .expect("repo lock")
            .set_step_status(run_id, step, status);
        Ok(())
    }

    async fn record_artifact(&self, _artifact: NewArtifact<'_>) -> Result<(), RepoError> {
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
            .insert(video.run_id, published.clone());
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

struct MockVideoProvider {
    events: Events,
}

#[async_trait]
impl VideoProvider for MockVideoProvider {
    async fn submit_prompt(
        &self,
        _request: VideoGenerationRequest,
    ) -> Result<VideoJob, VideoProviderError> {
        self.events.push("video:submit");
        Ok(VideoJob {
            provider: VideoProviderKind::GeminiVeo,
            provider_job_id: "video-job".to_owned(),
        })
    }

    async fn poll_job(&self, _job: &VideoJob) -> Result<VideoJobStatus, VideoProviderError> {
        self.events.push("video:poll");
        Ok(VideoJobStatus::Complete)
    }

    async fn download_clip(&self, _job: &VideoJob) -> Result<VideoClip, VideoProviderError> {
        self.events.push("video:download");
        Ok(VideoClip {
            bytes: b"raw-video".to_vec(),
            content_type: "video/mp4".to_owned(),
            file_extension: "mp4".to_owned(),
        })
    }
}

struct MockImageTo3DProvider {
    events: Events,
}

#[async_trait]
impl ImageTo3DProvider for MockImageTo3DProvider {
    async fn submit_image(
        &self,
        _request: ImageTo3DRequest,
    ) -> Result<ImageTo3DJob, ImageTo3DProviderError> {
        self.events.push("image_to_3d:submit");
        Ok(ImageTo3DJob {
            provider: ImageTo3DProviderKind::Meshy,
            provider_job_id: "mesh-job".to_owned(),
        })
    }

    async fn poll_job(
        &self,
        _job: &ImageTo3DJob,
    ) -> Result<ImageTo3DJobStatus, ImageTo3DProviderError> {
        self.events.push("image_to_3d:poll");
        Ok(ImageTo3DJobStatus::Complete)
    }

    async fn download_glb(&self, _job: &ImageTo3DJob) -> Result<GlbModel, ImageTo3DProviderError> {
        self.events.push("image_to_3d:download");
        Ok(GlbModel {
            bytes: b"glb".to_vec(),
            content_type: "model/gltf-binary".to_owned(),
            file_extension: "glb".to_owned(),
        })
    }
}

struct MockStorage {
    events: Events,
}

#[async_trait]
impl ArtifactStorage for MockStorage {
    async fn upload_artifact(
        &self,
        relative_key: String,
        _bytes: Vec<u8>,
        _content_type: Option<&str>,
    ) -> Result<StoredObject, StorageError> {
        self.events
            .push(format!("storage:upload:{}", file_name(&relative_key)));
        Ok(StoredObject {
            storage_key: format!("test-prefix/{relative_key}"),
            relative_key,
        })
    }

    async fn public_url(
        &self,
        relative_key: &str,
        _expires_in: Duration,
    ) -> Result<String, StorageError> {
        self.events
            .push(format!("storage:public_url:{}", file_name(relative_key)));
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

struct MockMedia {
    events: Events,
}

#[async_trait]
impl PipelineMedia for MockMedia {
    async fn extract_frame(&self, _input: &Path, output: &Path) -> Result<String, PipelineError> {
        self.events.push("media:extract_frame");
        fs::write(output, b"frame")?;
        Ok("image/jpeg".to_owned())
    }

    async fn render_reveal(&self, _input: &Path, output: &Path) -> Result<String, PipelineError> {
        self.events.push("media:render_reveal");
        fs::write(output, b"reveal")?;
        Ok("video/mp4".to_owned())
    }

    async fn assemble_final_video(
        &self,
        _funny_video: &Path,
        _reveal_clip: &Path,
        output: &Path,
    ) -> Result<(), PipelineError> {
        self.events.push("media:assemble");
        fs::write(output, b"final")?;
        Ok(())
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
