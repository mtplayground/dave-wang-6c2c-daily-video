use std::fmt;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::{
    models::{
        rotation::RotationAnimal,
        run::{Run, RunStatus},
    },
    pipeline::Pipeline,
    repo::{RepoError, Repository},
};

#[derive(Clone)]
pub struct AdminState {
    repo: Repository,
    pipeline: Pipeline,
}

impl AdminState {
    pub fn new(repo: Repository, pipeline: Pipeline) -> Self {
        Self { repo, pipeline }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRunRequest {
    date: NaiveDate,
    animal: RotationAnimal,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminRunResponse {
    run_id: Uuid,
    date: NaiveDate,
    animal: String,
    status: RunStatus,
    accepted: bool,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
}

pub async fn trigger_run(
    State(state): State<AdminState>,
    Json(request): Json<TriggerRunRequest>,
) -> Result<(StatusCode, Json<AdminRunResponse>), AdminError> {
    let animal = animal_slug(request.animal);
    let run = state.repo.create_run(request.date, animal).await?;
    spawn_pipeline_resume(state.pipeline, run.id);

    Ok((StatusCode::ACCEPTED, Json(AdminRunResponse::from_run(run))))
}

pub async fn retry_run(
    State(state): State<AdminState>,
    Path(run_id): Path<Uuid>,
) -> Result<(StatusCode, Json<AdminRunResponse>), AdminError> {
    let run = state
        .repo
        .get_run(run_id)
        .await?
        .ok_or(AdminError::RunNotFound)?;

    if run.status != RunStatus::Failed {
        return Err(AdminError::RunNotFailed);
    }

    spawn_pipeline_resume(state.pipeline, run.id);
    Ok((StatusCode::ACCEPTED, Json(AdminRunResponse::from_run(run))))
}

fn spawn_pipeline_resume(pipeline: Pipeline, run_id: Uuid) {
    tokio::spawn(async move {
        if let Err(error) = pipeline.resume_run(run_id).await {
            error!(%run_id, error = %error, "admin-triggered pipeline run failed");
        }
    });
}

impl AdminRunResponse {
    fn from_run(run: Run) -> Self {
        Self {
            run_id: run.id,
            date: run.date,
            animal: run.animal,
            status: run.status,
            accepted: true,
        }
    }
}

#[derive(Debug)]
pub enum AdminError {
    Repo(RepoError),
    RunNotFound,
    RunNotFailed,
}

impl fmt::Display for AdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repo(error) => write!(formatter, "repository error: {error}"),
            Self::RunNotFound => formatter.write_str("run not found"),
            Self::RunNotFailed => formatter.write_str("run is not failed"),
        }
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        match self {
            Self::RunNotFound => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "run not found",
                }),
            )
                .into_response(),
            Self::RunNotFailed => (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "only failed runs can be retried",
                }),
            )
                .into_response(),
            Self::Repo(error) => {
                error!(error = %error, "admin route repository operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "internal server error",
                    }),
                )
                    .into_response()
            }
        }
    }
}

impl From<RepoError> for AdminError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

fn animal_slug(animal: RotationAnimal) -> &'static str {
    match animal {
        RotationAnimal::Dog => "dog",
        RotationAnimal::Cat => "cat",
        RotationAnimal::Rabbit => "rabbit",
        RotationAnimal::Pig => "pig",
        RotationAnimal::Chicken => "chicken",
    }
}
