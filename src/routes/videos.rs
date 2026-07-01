use std::{fmt, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::{
    models::video::PublishedVideo,
    repo::{RepoError, Repository, normalize_pagination},
    storage::{ObjectStorage, StorageError},
};

const VIDEO_URL_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub struct VideosState {
    repo: Repository,
    storage: ObjectStorage,
}

impl VideosState {
    pub fn new(repo: Repository, storage: ObjectStorage) -> Self {
        Self { repo, storage }
    }
}

#[derive(Debug, Deserialize)]
pub struct VideosQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideosResponse {
    videos: Vec<VideoResponse>,
    pagination: PaginationResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestVideoResponse {
    video: VideoResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    limit: i64,
    offset: i64,
    count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoResponse {
    date: NaiveDate,
    animal: String,
    title: String,
    video_url: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
}

pub async fn list_videos(
    State(state): State<VideosState>,
    Query(query): Query<VideosQuery>,
) -> Result<Json<VideosResponse>, VideosError> {
    let (limit, offset) = normalize_pagination(query.limit.unwrap_or(20), query.offset.unwrap_or(0));
    let videos = state.repo.list_published_videos(limit, offset).await?;
    let mut response_videos = Vec::with_capacity(videos.len());

    for video in videos {
        response_videos.push(video_response(&state.storage, video).await?);
    }

    Ok(Json(VideosResponse {
        pagination: PaginationResponse {
            limit,
            offset,
            count: response_videos.len(),
        },
        videos: response_videos,
    }))
}

pub async fn latest_video(
    State(state): State<VideosState>,
) -> Result<Json<LatestVideoResponse>, VideosError> {
    let Some(video) = state.repo.list_published_videos(1, 0).await?.into_iter().next() else {
        return Err(VideosError::NotFound);
    };

    Ok(Json(LatestVideoResponse {
        video: video_response(&state.storage, video).await?,
    }))
}

async fn video_response(
    storage: &ObjectStorage,
    video: PublishedVideo,
) -> Result<VideoResponse, VideosError> {
    let video_url = storage
        .public_url_for_storage_key(&video.final_video_storage_key, VIDEO_URL_TTL)
        .await?;

    Ok(VideoResponse {
        date: video.date,
        animal: video.animal,
        title: video.title,
        video_url,
    })
}

#[derive(Debug)]
pub enum VideosError {
    Repo(RepoError),
    Storage(StorageError),
    NotFound,
}

impl fmt::Display for VideosError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repo(error) => write!(formatter, "repository error: {error}"),
            Self::Storage(error) => write!(formatter, "object storage error: {error}"),
            Self::NotFound => formatter.write_str("published video not found"),
        }
    }
}

impl IntoResponse for VideosError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "published video not found",
                }),
            )
                .into_response(),
            Self::Repo(error) => {
                error!(error = %error, "failed to read published videos");
                internal_server_error()
            }
            Self::Storage(error) => {
                error!(error = %error, "failed to sign published video URL");
                internal_server_error()
            }
        }
    }
}

impl From<RepoError> for VideosError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

impl From<StorageError> for VideosError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

fn internal_server_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "internal server error",
        }),
    )
        .into_response()
}
