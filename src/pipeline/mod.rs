use std::{
    error::Error,
    fmt,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use chrono::NaiveDate;
use tokio::time::sleep;
use uuid::Uuid;

use crate::{
    media::{
        assemble::{assemble_final_mp4_with_options, AssemblyError, AssemblyOptions},
        frame::{extract_representative_frame_with_options, FrameExtractionError, FrameExtractionOptions},
        render::{render_glb_turntable_with_options, RenderError, TurntableRenderOptions},
    },
    models::{
        artifact::ArtifactType,
        rotation::{RotationAnimal, RotationState},
        run::{PipelineStep, PipelineStepStatus, Run, RunStatus},
        video::PublishedVideo,
    },
    providers::{
        image_to_3d::{
            GlbModel, ImageTo3DJob, ImageTo3DJobStatus, ImageTo3DProvider, ImageTo3DProviderError,
            ImageTo3DRequest,
        },
        video::{
            build_funny_video_prompt, VideoGenerationRequest, VideoJob, VideoJobStatus, VideoProvider,
            VideoProviderError,
        },
    },
    repo::{NewArtifact, NewPublishedVideo, RepoError, Repository, RunStatusUpdate},
    storage::{ArtifactKind, ObjectStorage, StorageError},
};

const SIGNED_FRAME_URL_TTL: Duration = Duration::from_secs(60 * 60);

#[async_trait]
pub trait ArtifactStorage: Send + Sync {
    async fn upload_artifact(
        &self,
        relative_key: String,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<crate::storage::StoredObject, StorageError>;

    async fn public_url(
        &self,
        relative_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError>;

    fn artifact_key(
        &self,
        kind: ArtifactKind,
        run_id: Uuid,
        file_name: &str,
    ) -> Result<String, StorageError>;
}

#[async_trait]
impl ArtifactStorage for ObjectStorage {
    async fn upload_artifact(
        &self,
        relative_key: String,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<crate::storage::StoredObject, StorageError> {
        ObjectStorage::upload_artifact(self, relative_key, bytes, content_type).await
    }

    async fn public_url(
        &self,
        relative_key: &str,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        ObjectStorage::public_url(self, relative_key, expires_in).await
    }

    fn artifact_key(
        &self,
        kind: ArtifactKind,
        run_id: Uuid,
        file_name: &str,
    ) -> Result<String, StorageError> {
        ObjectStorage::artifact_key(self, kind, run_id, file_name)
    }
}

#[async_trait]
pub trait PipelineMedia: Send + Sync {
    async fn extract_frame(&self, input: &Path, output: &Path) -> Result<String, PipelineError>;

    async fn render_reveal(&self, input: &Path, output: &Path) -> Result<String, PipelineError>;

    async fn assemble_final_video(
        &self,
        funny_video: &Path,
        reveal_clip: &Path,
        output: &Path,
    ) -> Result<(), PipelineError>;
}

#[derive(Clone, Debug, Default)]
pub struct FfmpegPipelineMedia {
    frame_options: FrameExtractionOptions,
    render_options: TurntableRenderOptions,
    assembly_options: AssemblyOptions,
}

#[async_trait]
impl PipelineMedia for FfmpegPipelineMedia {
    async fn extract_frame(&self, input: &Path, output: &Path) -> Result<String, PipelineError> {
        let frame = extract_representative_frame_with_options(input, output, &self.frame_options)?;
        Ok(frame.content_type.to_owned())
    }

    async fn render_reveal(&self, input: &Path, output: &Path) -> Result<String, PipelineError> {
        let clip = render_glb_turntable_with_options(input, output, &self.render_options)?;
        Ok(clip.content_type.to_owned())
    }

    async fn assemble_final_video(
        &self,
        funny_video: &Path,
        reveal_clip: &Path,
        output: &Path,
    ) -> Result<(), PipelineError> {
        assemble_final_mp4_with_options(
            funny_video,
            reveal_clip,
            output,
            &self.assembly_options,
        )?;
        Ok(())
    }
}

#[async_trait]
pub trait RunRepository: Send + Sync {
    async fn advance_rotation(&self) -> Result<RotationState, RepoError>;

    async fn create_run(&self, date: NaiveDate, animal: &str) -> Result<Run, RepoError>;

    async fn get_run(&self, id: Uuid) -> Result<Option<Run>, RepoError>;

    async fn update_run_status(
        &self,
        id: Uuid,
        current_status: RunStatus,
        update: RunStatusUpdate,
        error: Option<&str>,
    ) -> Result<Run, RepoError>;

    async fn upsert_step_state(
        &self,
        run_id: Uuid,
        step: PipelineStep,
        status: PipelineStepStatus,
        error: Option<&str>,
    ) -> Result<(), RepoError>;

    async fn record_artifact(&self, artifact: NewArtifact<'_>) -> Result<(), RepoError>;

    async fn record_published_video(
        &self,
        video: NewPublishedVideo<'_>,
    ) -> Result<PublishedVideo, RepoError>;
}

#[async_trait]
impl RunRepository for Repository {
    async fn advance_rotation(&self) -> Result<RotationState, RepoError> {
        Repository::advance_rotation(self).await
    }

    async fn create_run(&self, date: NaiveDate, animal: &str) -> Result<Run, RepoError> {
        Repository::create_run(self, date, animal).await
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<Run>, RepoError> {
        Repository::get_run(self, id).await
    }

    async fn update_run_status(
        &self,
        id: Uuid,
        current_status: RunStatus,
        update: RunStatusUpdate,
        error: Option<&str>,
    ) -> Result<Run, RepoError> {
        Repository::update_run_status(self, id, current_status, update, error).await
    }

    async fn upsert_step_state(
        &self,
        run_id: Uuid,
        step: PipelineStep,
        status: PipelineStepStatus,
        error: Option<&str>,
    ) -> Result<(), RepoError> {
        Repository::upsert_step_state(self, run_id, step, status, error)
            .await
            .map(|_| ())
    }

    async fn record_artifact(&self, artifact: NewArtifact<'_>) -> Result<(), RepoError> {
        Repository::record_artifact(self, artifact).await.map(|_| ())
    }

    async fn record_published_video(
        &self,
        video: NewPublishedVideo<'_>,
    ) -> Result<PublishedVideo, RepoError> {
        Repository::record_published_video(self, video).await
    }
}

#[derive(Clone)]
pub struct Pipeline {
    repo: Arc<dyn RunRepository>,
    storage: Arc<dyn ArtifactStorage>,
    media: Arc<dyn PipelineMedia>,
    video_provider: Arc<dyn VideoProvider>,
    image_to_3d_provider: Arc<dyn ImageTo3DProvider>,
    workspace_dir: PathBuf,
    poll_options: PipelinePollOptions,
}

impl Pipeline {
    pub fn new(
        repo: Repository,
        storage: ObjectStorage,
        video_provider: Arc<dyn VideoProvider>,
        image_to_3d_provider: Arc<dyn ImageTo3DProvider>,
        workspace_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo: Arc::new(repo),
            storage: Arc::new(storage),
            media: Arc::new(FfmpegPipelineMedia::default()),
            video_provider,
            image_to_3d_provider,
            workspace_dir: workspace_dir.into(),
            poll_options: PipelinePollOptions::default(),
        }
    }

    pub fn with_components(
        repo: Arc<dyn RunRepository>,
        storage: Arc<dyn ArtifactStorage>,
        media: Arc<dyn PipelineMedia>,
        video_provider: Arc<dyn VideoProvider>,
        image_to_3d_provider: Arc<dyn ImageTo3DProvider>,
        workspace_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo,
            storage,
            media,
            video_provider,
            image_to_3d_provider,
            workspace_dir: workspace_dir.into(),
            poll_options: PipelinePollOptions::default(),
        }
    }

    pub fn with_poll_options(mut self, poll_options: PipelinePollOptions) -> Self {
        self.poll_options = poll_options;
        self
    }

    pub fn with_media_options(
        mut self,
        frame_options: FrameExtractionOptions,
        render_options: TurntableRenderOptions,
        assembly_options: AssemblyOptions,
    ) -> Self {
        self.media = Arc::new(FfmpegPipelineMedia {
            frame_options,
            render_options,
            assembly_options,
        });
        self
    }

    pub async fn start_daily_run(&self, date: NaiveDate) -> Result<PipelineRunOutcome, PipelineError> {
        let rotation = self.repo.advance_rotation().await?;
        let animal = animal_slug(rotation.current_animal);
        let run = self.repo.create_run(date, &animal).await?;

        self.resume_run(run.id).await
    }

    pub async fn resume_run(&self, run_id: Uuid) -> Result<PipelineRunOutcome, PipelineError> {
        let Some(mut run) = self.repo.get_run(run_id).await? else {
            return Err(PipelineError::RunNotFound(run_id));
        };

        if run.status == RunStatus::Complete {
            return Ok(PipelineRunOutcome {
                run,
                published_video: None,
            });
        }

        let start_index = start_step_index(&run)?;
        let paths = PipelinePaths::new(&self.workspace_dir, run.id);
        paths.ensure_dir()?;

        let animal = parse_animal_slug(&run.animal)?;
        let mut context = PipelineContext::new(paths, animal);

        for step in &PipelineStep::ORDERED[start_index..] {
            self.mark_step_started(&mut run, *step).await?;

            if let Err(error) = self.execute_step(&run, *step, &mut context).await {
                self.mark_step_failed(&run, *step, &error).await?;
                return Err(error);
            }

            self.repo
                .upsert_step_state(run.id, *step, PipelineStepStatus::Complete, None)
                .await?;
        }

        let complete_run = self
            .repo
            .update_run_status(
                run.id,
                run.status,
                RunStatusUpdate {
                    status: RunStatus::Complete,
                    current_step: None,
                },
                None,
            )
            .await?;

        Ok(PipelineRunOutcome {
            run: complete_run,
            published_video: context.published_video,
        })
    }

    async fn mark_step_started(&self, run: &mut Run, step: PipelineStep) -> Result<(), PipelineError> {
        let updated_run = self
            .repo
            .update_run_status(
                run.id,
                run.status,
                RunStatusUpdate {
                    status: RunStatus::InProgress,
                    current_step: Some(step),
                },
                None,
            )
            .await?;

        self.repo
            .upsert_step_state(run.id, step, PipelineStepStatus::InProgress, None)
            .await?;

        *run = updated_run;
        Ok(())
    }

    async fn mark_step_failed(
        &self,
        run: &Run,
        step: PipelineStep,
        error: &PipelineError,
    ) -> Result<(), PipelineError> {
        let message = error.to_string();

        self.repo
            .upsert_step_state(run.id, step, PipelineStepStatus::Failed, Some(&message))
            .await?;

        self.repo
            .update_run_status(
                run.id,
                run.status,
                RunStatusUpdate {
                    status: RunStatus::Failed,
                    current_step: Some(step),
                },
                Some(&message),
            )
            .await?;

        Ok(())
    }

    async fn execute_step(
        &self,
        run: &Run,
        step: PipelineStep,
        context: &mut PipelineContext,
    ) -> Result<(), PipelineError> {
        match step {
            PipelineStep::PickAnimal => Ok(()),
            PipelineStep::GenerateVideo => self.generate_video(run, context).await,
            PipelineStep::ExtractFrame => self.extract_frame(run, context).await,
            PipelineStep::ImageTo3D => self.image_to_3d(run, context).await,
            PipelineStep::RenderReveal => self.render_reveal(run, context).await,
            PipelineStep::Assemble => self.assemble_final_video(context).await,
            PipelineStep::Upload => self.upload_final_video(run, context).await,
            PipelineStep::RecordPublishedVideo => self.record_published_video(run, context).await,
        }
    }

    async fn generate_video(&self, run: &Run, context: &mut PipelineContext) -> Result<(), PipelineError> {
        let prompt = build_funny_video_prompt(context.animal);
        let job = self
            .video_provider
            .submit_prompt(VideoGenerationRequest::new(prompt))
            .await?;

        self.wait_for_video_job(&job).await?;
        let clip = self.video_provider.download_clip(&job).await?;

        fs::write(&context.paths.raw_video, &clip.bytes)?;
        let artifact = self
            .upload_artifact(
                run.id,
                ArtifactKind::RawVideo,
                ArtifactType::RawVideo,
                "raw.mp4",
                clip.bytes,
                Some(&clip.content_type),
            )
            .await?;

        context.raw_video = Some(artifact);
        Ok(())
    }

    async fn wait_for_video_job(&self, job: &VideoJob) -> Result<(), PipelineError> {
        for attempt in 0..self.poll_options.max_attempts {
            match self.video_provider.poll_job(job).await? {
                VideoJobStatus::Complete => return Ok(()),
                VideoJobStatus::Failed { message } => {
                    return Err(PipelineError::ProviderFailed {
                        step: PipelineStep::GenerateVideo,
                        message,
                    });
                }
                VideoJobStatus::Pending | VideoJobStatus::Running => {
                    if attempt + 1 < self.poll_options.max_attempts {
                        sleep(self.poll_options.interval).await;
                    }
                }
            }
        }

        Err(PipelineError::ProviderTimeout {
            step: PipelineStep::GenerateVideo,
        })
    }

    async fn extract_frame(&self, run: &Run, context: &mut PipelineContext) -> Result<(), PipelineError> {
        ensure_file_exists(&context.paths.raw_video, PipelineStep::ExtractFrame)?;

        let content_type = self.media.extract_frame(
            &context.paths.raw_video,
            &context.paths.frame,
        ).await?;
        let bytes = fs::read(&context.paths.frame)?;
        let artifact = self
            .upload_artifact(
                run.id,
                ArtifactKind::Frame,
                ArtifactType::Frame,
                "frame.jpg",
                bytes,
                Some(&content_type),
            )
            .await?;

        context.frame = Some(artifact);
        Ok(())
    }

    async fn image_to_3d(&self, run: &Run, context: &mut PipelineContext) -> Result<(), PipelineError> {
        let frame = self.ensure_frame_artifact(run, context).await?;
        let image_url = self
            .storage
            .public_url(&frame.relative_key, SIGNED_FRAME_URL_TTL)
            .await?;
        let job = self
            .image_to_3d_provider
            .submit_image(ImageTo3DRequest::new(image_url))
            .await?;

        self.wait_for_image_to_3d_job(&job).await?;
        let model = self.image_to_3d_provider.download_glb(&job).await?;
        self.persist_glb(run, context, model).await
    }

    async fn ensure_frame_artifact(
        &self,
        run: &Run,
        context: &mut PipelineContext,
    ) -> Result<PipelineArtifact, PipelineError> {
        if let Some(frame) = &context.frame {
            return Ok(frame.clone());
        }

        ensure_file_exists(&context.paths.frame, PipelineStep::ImageTo3D)?;
        let bytes = fs::read(&context.paths.frame)?;
        let artifact = self
            .upload_artifact(
                run.id,
                ArtifactKind::Frame,
                ArtifactType::Frame,
                "frame.jpg",
                bytes,
                Some("image/jpeg"),
            )
            .await?;

        context.frame = Some(artifact.clone());
        Ok(artifact)
    }

    async fn wait_for_image_to_3d_job(&self, job: &ImageTo3DJob) -> Result<(), PipelineError> {
        for attempt in 0..self.poll_options.max_attempts {
            match self.image_to_3d_provider.poll_job(job).await? {
                ImageTo3DJobStatus::Complete => return Ok(()),
                ImageTo3DJobStatus::Failed { message } => {
                    return Err(PipelineError::ProviderFailed {
                        step: PipelineStep::ImageTo3D,
                        message,
                    });
                }
                ImageTo3DJobStatus::Pending | ImageTo3DJobStatus::Running { .. } => {
                    if attempt + 1 < self.poll_options.max_attempts {
                        sleep(self.poll_options.interval).await;
                    }
                }
            }
        }

        Err(PipelineError::ProviderTimeout {
            step: PipelineStep::ImageTo3D,
        })
    }

    async fn persist_glb(
        &self,
        run: &Run,
        context: &mut PipelineContext,
        model: GlbModel,
    ) -> Result<(), PipelineError> {
        fs::write(&context.paths.glb, &model.bytes)?;
        let artifact = self
            .upload_artifact(
                run.id,
                ArtifactKind::Glb,
                ArtifactType::Glb,
                "model.glb",
                model.bytes,
                Some(&model.content_type),
            )
            .await?;

        context.glb = Some(artifact);
        Ok(())
    }

    async fn render_reveal(&self, run: &Run, context: &mut PipelineContext) -> Result<(), PipelineError> {
        ensure_file_exists(&context.paths.glb, PipelineStep::RenderReveal)?;

        let content_type = self.media.render_reveal(
            &context.paths.glb,
            &context.paths.reveal_clip,
        ).await?;
        let bytes = fs::read(&context.paths.reveal_clip)?;
        let artifact = self
            .upload_artifact(
                run.id,
                ArtifactKind::RevealClip,
                ArtifactType::RevealClip,
                "reveal.mp4",
                bytes,
                Some(&content_type),
            )
            .await?;

        context.reveal_clip = Some(artifact);
        Ok(())
    }

    async fn assemble_final_video(&self, context: &mut PipelineContext) -> Result<(), PipelineError> {
        ensure_file_exists(&context.paths.raw_video, PipelineStep::Assemble)?;
        ensure_file_exists(&context.paths.reveal_clip, PipelineStep::Assemble)?;

        self.media.assemble_final_video(
            &context.paths.raw_video,
            &context.paths.reveal_clip,
            &context.paths.final_mp4,
        ).await?;

        Ok(())
    }

    async fn upload_final_video(&self, run: &Run, context: &mut PipelineContext) -> Result<(), PipelineError> {
        ensure_file_exists(&context.paths.final_mp4, PipelineStep::Upload)?;

        let bytes = fs::read(&context.paths.final_mp4)?;
        let artifact = self
            .upload_artifact(
                run.id,
                ArtifactKind::FinalMp4,
                ArtifactType::FinalMp4,
                "final.mp4",
                bytes,
                Some("video/mp4"),
            )
            .await?;

        context.final_mp4 = Some(artifact);
        Ok(())
    }

    async fn record_published_video(
        &self,
        run: &Run,
        context: &mut PipelineContext,
    ) -> Result<(), PipelineError> {
        if context.final_mp4.is_none() {
            self.upload_final_video(run, context).await?;
        }

        let final_mp4 = context
            .final_mp4
            .as_ref()
            .ok_or(PipelineError::MissingArtifact {
                step: PipelineStep::RecordPublishedVideo,
                artifact: "final_mp4",
            })?;
        let title = format!("Daily {} 3D print reveal", display_animal(context.animal));

        let video = self
            .repo
            .record_published_video(NewPublishedVideo {
                run_id: run.id,
                date: run.date,
                animal: &run.animal,
                title: &title,
                final_video_storage_key: &final_mp4.storage_key,
            })
            .await?;

        context.published_video = Some(video);
        Ok(())
    }

    async fn upload_artifact(
        &self,
        run_id: Uuid,
        kind: ArtifactKind,
        artifact_type: ArtifactType,
        file_name: &str,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<PipelineArtifact, PipelineError> {
        let byte_size = bytes.len() as i64;
        let relative_key = self.storage.artifact_key(kind, run_id, file_name)?;
        let stored = self
            .storage
            .upload_artifact(relative_key.clone(), bytes, content_type)
            .await?;

        self.repo
            .record_artifact(NewArtifact {
                run_id,
                artifact_type,
                storage_key: &stored.storage_key,
                content_type,
                byte_size: Some(byte_size),
            })
            .await?;

        Ok(PipelineArtifact {
            relative_key: stored.relative_key,
            storage_key: stored.storage_key,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelineRunOutcome {
    pub run: Run,
    pub published_video: Option<PublishedVideo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelinePollOptions {
    pub max_attempts: u32,
    pub interval: Duration,
}

impl Default for PipelinePollOptions {
    fn default() -> Self {
        Self {
            max_attempts: 180,
            interval: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PipelineArtifact {
    relative_key: String,
    storage_key: String,
}

#[derive(Debug)]
struct PipelineContext {
    paths: PipelinePaths,
    animal: RotationAnimal,
    raw_video: Option<PipelineArtifact>,
    frame: Option<PipelineArtifact>,
    glb: Option<PipelineArtifact>,
    reveal_clip: Option<PipelineArtifact>,
    final_mp4: Option<PipelineArtifact>,
    published_video: Option<PublishedVideo>,
}

impl PipelineContext {
    fn new(paths: PipelinePaths, animal: RotationAnimal) -> Self {
        Self {
            paths,
            animal,
            raw_video: None,
            frame: None,
            glb: None,
            reveal_clip: None,
            final_mp4: None,
            published_video: None,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PipelinePaths {
    run_dir: PathBuf,
    raw_video: PathBuf,
    frame: PathBuf,
    glb: PathBuf,
    reveal_clip: PathBuf,
    final_mp4: PathBuf,
}

impl PipelinePaths {
    fn new(workspace_dir: &Path, run_id: Uuid) -> Self {
        let run_dir = workspace_dir.join(run_id.to_string());
        Self {
            raw_video: run_dir.join("raw.mp4"),
            frame: run_dir.join("frame.jpg"),
            glb: run_dir.join("model.glb"),
            reveal_clip: run_dir.join("reveal.mp4"),
            final_mp4: run_dir.join("final.mp4"),
            run_dir,
        }
    }

    fn ensure_dir(&self) -> Result<(), PipelineError> {
        fs::create_dir_all(&self.run_dir)?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum PipelineError {
    Repo(RepoError),
    Storage(StorageError),
    VideoProvider(VideoProviderError),
    ImageTo3DProvider(ImageTo3DProviderError),
    Frame(FrameExtractionError),
    Render(RenderError),
    Assembly(AssemblyError),
    Io(std::io::Error),
    RunNotFound(Uuid),
    InvalidAnimal(String),
    InvalidRunState(String),
    MissingLocalFile {
        step: PipelineStep,
        path: PathBuf,
    },
    MissingArtifact {
        step: PipelineStep,
        artifact: &'static str,
    },
    ProviderFailed {
        step: PipelineStep,
        message: String,
    },
    ProviderTimeout {
        step: PipelineStep,
    },
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::Repo(error) => write!(f, "repository error: {error}"),
            PipelineError::Storage(error) => write!(f, "object storage error: {error}"),
            PipelineError::VideoProvider(error) => write!(f, "video provider error: {error}"),
            PipelineError::ImageTo3DProvider(error) => {
                write!(f, "image-to-3d provider error: {error}")
            }
            PipelineError::Frame(error) => write!(f, "frame extraction error: {error}"),
            PipelineError::Render(error) => write!(f, "turntable render error: {error}"),
            PipelineError::Assembly(error) => write!(f, "final assembly error: {error}"),
            PipelineError::Io(error) => write!(f, "filesystem error: {error}"),
            PipelineError::RunNotFound(run_id) => write!(f, "run {run_id} was not found"),
            PipelineError::InvalidAnimal(animal) => write!(f, "invalid rotation animal: {animal}"),
            PipelineError::InvalidRunState(message) => write!(f, "invalid run state: {message}"),
            PipelineError::MissingLocalFile { step, path } => {
                write!(f, "missing local prerequisite for {step:?}: {}", path.display())
            }
            PipelineError::MissingArtifact { step, artifact } => {
                write!(f, "missing {artifact} artifact for {step:?}")
            }
            PipelineError::ProviderFailed { step, message } => {
                write!(f, "provider failed during {step:?}: {message}")
            }
            PipelineError::ProviderTimeout { step } => {
                write!(f, "provider timed out during {step:?}")
            }
        }
    }
}

impl Error for PipelineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PipelineError::Repo(error) => Some(error),
            PipelineError::Storage(error) => Some(error),
            PipelineError::VideoProvider(error) => Some(error),
            PipelineError::ImageTo3DProvider(error) => Some(error),
            PipelineError::Frame(error) => Some(error),
            PipelineError::Render(error) => Some(error),
            PipelineError::Assembly(error) => Some(error),
            PipelineError::Io(error) => Some(error),
            PipelineError::RunNotFound(_)
            | PipelineError::InvalidAnimal(_)
            | PipelineError::InvalidRunState(_)
            | PipelineError::MissingLocalFile { .. }
            | PipelineError::MissingArtifact { .. }
            | PipelineError::ProviderFailed { .. }
            | PipelineError::ProviderTimeout { .. } => None,
        }
    }
}

impl From<RepoError> for PipelineError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

impl From<StorageError> for PipelineError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<VideoProviderError> for PipelineError {
    fn from(error: VideoProviderError) -> Self {
        Self::VideoProvider(error)
    }
}

impl From<ImageTo3DProviderError> for PipelineError {
    fn from(error: ImageTo3DProviderError) -> Self {
        Self::ImageTo3DProvider(error)
    }
}

impl From<FrameExtractionError> for PipelineError {
    fn from(error: FrameExtractionError) -> Self {
        Self::Frame(error)
    }
}

impl From<RenderError> for PipelineError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl From<AssemblyError> for PipelineError {
    fn from(error: AssemblyError) -> Self {
        Self::Assembly(error)
    }
}

impl From<std::io::Error> for PipelineError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

fn start_step_index(run: &Run) -> Result<usize, PipelineError> {
    match run.status {
        RunStatus::Complete => Ok(PipelineStep::ORDERED.len()),
        RunStatus::Pending => Ok(0),
        RunStatus::InProgress | RunStatus::Failed => {
            let Some(step) = run.current_step else {
                return Ok(0);
            };

            PipelineStep::ORDERED
                .iter()
                .position(|candidate| *candidate == step)
                .ok_or_else(|| PipelineError::InvalidRunState(format!("unknown step {step:?}")))
        }
    }
}

fn ensure_file_exists(path: &Path, step: PipelineStep) -> Result<(), PipelineError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(PipelineError::MissingLocalFile {
            step,
            path: path.to_path_buf(),
        })
    }
}

fn parse_animal_slug(animal: &str) -> Result<RotationAnimal, PipelineError> {
    match animal {
        "dog" => Ok(RotationAnimal::Dog),
        "cat" => Ok(RotationAnimal::Cat),
        "rabbit" => Ok(RotationAnimal::Rabbit),
        "pig" => Ok(RotationAnimal::Pig),
        "chicken" => Ok(RotationAnimal::Chicken),
        value => Err(PipelineError::InvalidAnimal(value.to_owned())),
    }
}

fn animal_slug(animal: RotationAnimal) -> String {
    match animal {
        RotationAnimal::Dog => "dog",
        RotationAnimal::Cat => "cat",
        RotationAnimal::Rabbit => "rabbit",
        RotationAnimal::Pig => "pig",
        RotationAnimal::Chicken => "chicken",
    }
    .to_owned()
}

fn display_animal(animal: RotationAnimal) -> &'static str {
    match animal {
        RotationAnimal::Dog => "dog",
        RotationAnimal::Cat => "cat",
        RotationAnimal::Rabbit => "rabbit",
        RotationAnimal::Pig => "pig",
        RotationAnimal::Chicken => "chicken",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_state(status: RunStatus, current_step: Option<PipelineStep>) -> Run {
        Run {
            id: Uuid::nil(),
            date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
            animal: "dog".to_owned(),
            status,
            current_step,
            error: None,
            created_at: chrono::DateTime::UNIX_EPOCH,
            updated_at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn failed_run_resumes_at_failed_step() {
        let run = run_with_state(RunStatus::Failed, Some(PipelineStep::RenderReveal));

        assert_eq!(
            start_step_index(&run).expect("step index"),
            PipelineStep::ORDERED
                .iter()
                .position(|step| *step == PipelineStep::RenderReveal)
                .expect("render step in ordered list")
        );
    }

    #[test]
    fn pending_run_starts_from_first_step() {
        let run = run_with_state(RunStatus::Pending, None);

        assert_eq!(start_step_index(&run).expect("step index"), 0);
    }

    #[test]
    fn parses_supported_rotation_animals() {
        assert_eq!(parse_animal_slug("dog").expect("dog"), RotationAnimal::Dog);
        assert_eq!(parse_animal_slug("cat").expect("cat"), RotationAnimal::Cat);
        assert_eq!(parse_animal_slug("rabbit").expect("rabbit"), RotationAnimal::Rabbit);
        assert_eq!(parse_animal_slug("pig").expect("pig"), RotationAnimal::Pig);
        assert_eq!(
            parse_animal_slug("chicken").expect("chicken"),
            RotationAnimal::Chicken
        );
    }

    #[test]
    fn rejects_unknown_rotation_animal() {
        assert!(matches!(
            parse_animal_slug("horse"),
            Err(PipelineError::InvalidAnimal(value)) if value == "horse"
        ));
    }

}
