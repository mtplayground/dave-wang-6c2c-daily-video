#![allow(dead_code)]

use std::{
    error::Error,
    fmt::{self, Display},
};

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{
    artifact::{Artifact, ArtifactType},
    rotation::{ROTATION_STATE_KEY, RotationAnimal, RotationState},
    run::{PipelineStep, PipelineStepStatus, Run, RunStatus, RunStepState},
    video::PublishedVideo,
};

#[derive(Debug, Clone)]
pub struct Repository {
    pool: PgPool,
}

#[derive(Debug)]
pub enum RepoError {
    Database(sqlx::Error),
    InvalidRunStatusTransition(InvalidRunStatusTransition),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRunStatusTransition {
    pub from: RunStatus,
    pub to: RunStatus,
}

#[derive(Debug, Clone, Copy)]
pub struct RunStatusUpdate {
    pub status: RunStatus,
    pub current_step: Option<PipelineStep>,
}

#[derive(Debug, Clone, Copy)]
pub struct StepStatusUpdate {
    pub status: PipelineStepStatus,
    pub error: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct NewArtifact<'a> {
    pub run_id: Uuid,
    pub artifact_type: ArtifactType,
    pub storage_key: &'a str,
    pub content_type: Option<&'a str>,
    pub byte_size: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct NewPublishedVideo<'a> {
    pub run_id: Uuid,
    pub date: NaiveDate,
    pub animal: &'a str,
    pub title: &'a str,
    pub final_video_storage_key: &'a str,
}

impl Repository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_run(&self, date: NaiveDate, animal: &str) -> Result<Run, RepoError> {
        sqlx::query_as::<_, Run>(
            r#"
            INSERT INTO runs (date, animal)
            VALUES ($1, $2)
            RETURNING id, date, animal, status, current_step, error, created_at, updated_at
            "#,
        )
        .bind(date)
        .bind(animal)
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn get_run(&self, id: Uuid) -> Result<Option<Run>, RepoError> {
        sqlx::query_as::<_, Run>(
            r#"
            SELECT id, date, animal, status, current_step, error, created_at, updated_at
            FROM runs
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn update_run_status(
        &self,
        id: Uuid,
        current_status: RunStatus,
        update: RunStatusUpdate,
        error: Option<&str>,
    ) -> Result<Run, RepoError> {
        validate_run_status_transition(current_status, update.status)?;

        sqlx::query_as::<_, Run>(
            r#"
            UPDATE runs
            SET status = $2,
                current_step = $3,
                error = $4
            WHERE id = $1
            RETURNING id, date, animal, status, current_step, error, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(update.status)
        .bind(update.current_step)
        .bind(error)
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn upsert_step_state(
        &self,
        run_id: Uuid,
        step: PipelineStep,
        status: PipelineStepStatus,
        error: Option<&str>,
    ) -> Result<RunStepState, RepoError> {
        sqlx::query_as::<_, RunStepState>(
            r#"
            INSERT INTO run_step_states (run_id, step, status, attempt_count, error, started_at, completed_at)
            VALUES (
                $1,
                $2,
                $3,
                CASE WHEN $3 = 'in_progress'::pipeline_step_status THEN 1 ELSE 0 END,
                $4,
                CASE WHEN $3 = 'in_progress'::pipeline_step_status THEN NOW() ELSE NULL END,
                CASE WHEN $3 = 'complete'::pipeline_step_status THEN NOW() ELSE NULL END
            )
            ON CONFLICT (run_id, step)
            DO UPDATE SET
                status = EXCLUDED.status,
                attempt_count = run_step_states.attempt_count
                    + CASE WHEN EXCLUDED.status = 'in_progress'::pipeline_step_status THEN 1 ELSE 0 END,
                error = EXCLUDED.error,
                started_at = CASE
                    WHEN EXCLUDED.status = 'in_progress'::pipeline_step_status THEN COALESCE(run_step_states.started_at, NOW())
                    ELSE run_step_states.started_at
                END,
                completed_at = CASE
                    WHEN EXCLUDED.status = 'complete'::pipeline_step_status THEN NOW()
                    ELSE NULL
                END
            RETURNING run_id, step, status, attempt_count, error, started_at, completed_at, created_at, updated_at
            "#,
        )
        .bind(run_id)
        .bind(step)
        .bind(status)
        .bind(error)
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn get_rotation_state(&self) -> Result<RotationState, RepoError> {
        sqlx::query_as::<_, RotationState>(
            r#"
            SELECT key, current_position, current_animal, created_at, updated_at
            FROM rotation_state
            WHERE key = $1
            "#,
        )
        .bind(ROTATION_STATE_KEY)
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn advance_rotation(&self) -> Result<RotationState, RepoError> {
        let mut tx = self.pool.begin().await.map_err(RepoError::Database)?;

        let current = sqlx::query_as::<_, RotationState>(
            r#"
            SELECT key, current_position, current_animal, created_at, updated_at
            FROM rotation_state
            WHERE key = $1
            FOR UPDATE
            "#,
        )
        .bind(ROTATION_STATE_KEY)
        .fetch_one(&mut *tx)
        .await
        .map_err(RepoError::Database)?;

        let (next_position, next_animal) = advance_rotation_value(current.current_animal);

        let updated = sqlx::query_as::<_, RotationState>(
            r#"
            UPDATE rotation_state
            SET current_position = $2,
                current_animal = $3
            WHERE key = $1
            RETURNING key, current_position, current_animal, created_at, updated_at
            "#,
        )
        .bind(ROTATION_STATE_KEY)
        .bind(next_position)
        .bind(next_animal)
        .fetch_one(&mut *tx)
        .await
        .map_err(RepoError::Database)?;

        tx.commit().await.map_err(RepoError::Database)?;
        Ok(updated)
    }

    pub async fn record_artifact(&self, artifact: NewArtifact<'_>) -> Result<Artifact, RepoError> {
        sqlx::query_as::<_, Artifact>(
            r#"
            INSERT INTO artifacts (run_id, artifact_type, storage_key, content_type, byte_size)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (run_id, artifact_type)
            DO UPDATE SET
                storage_key = EXCLUDED.storage_key,
                content_type = EXCLUDED.content_type,
                byte_size = EXCLUDED.byte_size
            RETURNING id, run_id, artifact_type, storage_key, content_type, byte_size, created_at, updated_at
            "#,
        )
        .bind(artifact.run_id)
        .bind(artifact.artifact_type)
        .bind(artifact.storage_key)
        .bind(artifact.content_type)
        .bind(artifact.byte_size)
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn record_published_video(
        &self,
        video: NewPublishedVideo<'_>,
    ) -> Result<PublishedVideo, RepoError> {
        sqlx::query_as::<_, PublishedVideo>(
            r#"
            INSERT INTO published_videos (run_id, date, animal, title, final_video_storage_key)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (date)
            DO UPDATE SET
                run_id = EXCLUDED.run_id,
                animal = EXCLUDED.animal,
                title = EXCLUDED.title,
                final_video_storage_key = EXCLUDED.final_video_storage_key,
                published_at = NOW()
            RETURNING id, run_id, date, animal, title, final_video_storage_key, published_at, created_at, updated_at
            "#,
        )
        .bind(video.run_id)
        .bind(video.date)
        .bind(video.animal)
        .bind(video.title)
        .bind(video.final_video_storage_key)
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::Database)
    }

    pub async fn list_published_videos(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PublishedVideo>, RepoError> {
        let (limit, offset) = normalize_pagination(limit, offset);

        sqlx::query_as::<_, PublishedVideo>(
            r#"
            SELECT id, run_id, date, animal, title, final_video_storage_key, published_at, created_at, updated_at
            FROM published_videos
            ORDER BY date DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::Database)
    }
}

pub fn advance_rotation_value(current: RotationAnimal) -> (i16, RotationAnimal) {
    let next = current.next();
    (next.position(), next)
}

pub fn validate_run_status_transition(
    from: RunStatus,
    to: RunStatus,
) -> Result<(), InvalidRunStatusTransition> {
    if from == to {
        return Ok(());
    }

    match (from, to) {
        (RunStatus::Pending, RunStatus::InProgress)
        | (RunStatus::Pending, RunStatus::Failed)
        | (RunStatus::InProgress, RunStatus::Complete)
        | (RunStatus::InProgress, RunStatus::Failed)
        | (RunStatus::Failed, RunStatus::Pending)
        | (RunStatus::Failed, RunStatus::InProgress) => Ok(()),
        _ => Err(InvalidRunStatusTransition { from, to }),
    }
}

pub fn normalize_pagination(limit: i64, offset: i64) -> (i64, i64) {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    (limit, offset)
}

impl Display for RepoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(_) => write!(formatter, "repository database operation failed"),
            Self::InvalidRunStatusTransition(err) => Display::fmt(err, formatter),
        }
    }
}

impl Error for RepoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            Self::InvalidRunStatusTransition(err) => Some(err),
        }
    }
}

impl From<sqlx::Error> for RepoError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}

impl From<InvalidRunStatusTransition> for RepoError {
    fn from(err: InvalidRunStatusTransition) -> Self {
        Self::InvalidRunStatusTransition(err)
    }
}

impl Display for InvalidRunStatusTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid run status transition from {:?} to {:?}",
            self.from, self.to
        )
    }
}

impl Error for InvalidRunStatusTransition {}
