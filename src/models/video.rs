#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PublishedVideo {
    pub id: Uuid,
    pub run_id: Uuid,
    pub date: NaiveDate,
    pub animal: String,
    pub title: String,
    pub final_video_storage_key: String,
    pub published_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
