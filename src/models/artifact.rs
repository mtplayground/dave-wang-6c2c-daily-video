#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "artifact_type")]
pub enum ArtifactType {
    #[sqlx(rename = "raw_video")]
    #[serde(rename = "raw_video")]
    RawVideo,
    #[sqlx(rename = "frame")]
    #[serde(rename = "frame")]
    Frame,
    #[sqlx(rename = "glb")]
    #[serde(rename = "glb")]
    Glb,
    #[sqlx(rename = "reveal_clip")]
    #[serde(rename = "reveal_clip")]
    RevealClip,
    #[sqlx(rename = "final_mp4")]
    #[serde(rename = "final_mp4")]
    FinalMp4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Artifact {
    pub id: Uuid,
    pub run_id: Uuid,
    pub artifact_type: ArtifactType,
    pub storage_key: String,
    pub content_type: Option<String>,
    pub byte_size: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ArtifactType {
    pub const REQUIRED_FOR_PUBLISH: [Self; 5] = [
        Self::RawVideo,
        Self::Frame,
        Self::Glb,
        Self::RevealClip,
        Self::FinalMp4,
    ];
}
