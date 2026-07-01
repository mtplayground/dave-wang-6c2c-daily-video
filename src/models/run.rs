#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "run_status")]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[sqlx(rename = "pending")]
    Pending,
    #[sqlx(rename = "in_progress")]
    InProgress,
    #[sqlx(rename = "failed")]
    Failed,
    #[sqlx(rename = "complete")]
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "pipeline_step")]
pub enum PipelineStep {
    #[sqlx(rename = "pick_animal")]
    #[serde(rename = "pick_animal")]
    PickAnimal,
    #[sqlx(rename = "generate_video")]
    #[serde(rename = "generate_video")]
    GenerateVideo,
    #[sqlx(rename = "extract_frame")]
    #[serde(rename = "extract_frame")]
    ExtractFrame,
    #[sqlx(rename = "image_to_3d")]
    #[serde(rename = "image_to_3d")]
    ImageTo3D,
    #[sqlx(rename = "render_reveal")]
    #[serde(rename = "render_reveal")]
    RenderReveal,
    #[sqlx(rename = "assemble")]
    #[serde(rename = "assemble")]
    Assemble,
    #[sqlx(rename = "upload")]
    #[serde(rename = "upload")]
    Upload,
    #[sqlx(rename = "record_published_video")]
    #[serde(rename = "record_published_video")]
    RecordPublishedVideo,
}

impl PipelineStep {
    pub const ORDERED: [Self; 8] = [
        Self::PickAnimal,
        Self::GenerateVideo,
        Self::ExtractFrame,
        Self::ImageTo3D,
        Self::RenderReveal,
        Self::Assemble,
        Self::Upload,
        Self::RecordPublishedVideo,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "pipeline_step_status")]
#[serde(rename_all = "snake_case")]
pub enum PipelineStepStatus {
    #[sqlx(rename = "pending")]
    Pending,
    #[sqlx(rename = "in_progress")]
    InProgress,
    #[sqlx(rename = "failed")]
    Failed,
    #[sqlx(rename = "complete")]
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Run {
    pub id: Uuid,
    pub date: NaiveDate,
    pub animal: String,
    pub status: RunStatus,
    pub current_step: Option<PipelineStep>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct RunStepState {
    pub run_id: Uuid,
    pub step: PipelineStep,
    pub status: PipelineStepStatus,
    pub attempt_count: i32,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
