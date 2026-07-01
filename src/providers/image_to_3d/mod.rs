#![allow(dead_code)]

use std::{
    error::Error,
    fmt::{self, Display},
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::ProviderConfig;

pub mod meshy;

pub use meshy::MeshyImageTo3DProvider;

pub const DEFAULT_IMAGE_TO_3D_PROVIDER: ImageTo3DProviderKind = ImageTo3DProviderKind::Meshy;
pub const DEFAULT_IMAGE_TO_3D_POLL_INTERVAL: Duration = Duration::from_secs(10);

#[async_trait]
pub trait ImageTo3DProvider: Send + Sync {
    async fn submit_image(
        &self,
        request: ImageTo3DRequest,
    ) -> Result<ImageTo3DJob, ImageTo3DProviderError>;

    async fn poll_job(
        &self,
        job: &ImageTo3DJob,
    ) -> Result<ImageTo3DJobStatus, ImageTo3DProviderError>;

    async fn download_glb(&self, job: &ImageTo3DJob) -> Result<GlbModel, ImageTo3DProviderError>;
}

#[derive(Debug, Clone)]
pub struct UnavailableImageTo3DProvider {
    message: String,
}

impl UnavailableImageTo3DProvider {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
impl ImageTo3DProvider for UnavailableImageTo3DProvider {
    async fn submit_image(
        &self,
        _request: ImageTo3DRequest,
    ) -> Result<ImageTo3DJob, ImageTo3DProviderError> {
        Err(ImageTo3DProviderError::SubmitFailed {
            message: self.message.clone(),
        })
    }

    async fn poll_job(
        &self,
        _job: &ImageTo3DJob,
    ) -> Result<ImageTo3DJobStatus, ImageTo3DProviderError> {
        Err(ImageTo3DProviderError::PollFailed {
            message: self.message.clone(),
        })
    }

    async fn download_glb(&self, _job: &ImageTo3DJob) -> Result<GlbModel, ImageTo3DProviderError> {
        Err(ImageTo3DProviderError::DownloadFailed {
            message: self.message.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageTo3DRequest {
    pub image_url: String,
    pub texture_prompt: Option<String>,
    pub poll_interval: Duration,
    pub ai_model: String,
    pub model_type: MeshModelType,
    pub should_texture: bool,
    pub enable_pbr: bool,
    pub should_remesh: bool,
    pub target_polycount: u32,
    pub target_formats: Vec<String>,
}

impl ImageTo3DRequest {
    pub fn new(image_url: impl Into<String>) -> Self {
        Self {
            image_url: image_url.into(),
            texture_prompt: None,
            poll_interval: DEFAULT_IMAGE_TO_3D_POLL_INTERVAL,
            ai_model: "latest".to_owned(),
            model_type: MeshModelType::Standard,
            should_texture: true,
            enable_pbr: true,
            should_remesh: true,
            target_polycount: 100_000,
            target_formats: vec!["glb".to_owned()],
        }
    }

    pub fn with_texture_prompt(mut self, texture_prompt: impl Into<String>) -> Self {
        self.texture_prompt = Some(texture_prompt.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshModelType {
    Standard,
    Lowpoly,
}

impl MeshModelType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Lowpoly => "lowpoly",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageTo3DJob {
    pub provider: ImageTo3DProviderKind,
    pub provider_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageTo3DJobStatus {
    Pending,
    Running { progress: Option<u8> },
    Complete,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlbModel {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub file_extension: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageTo3DProviderKind {
    Meshy,
}

impl ImageTo3DProviderKind {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, ImageTo3DProviderError> {
        Self::from_name(&config.image_to_3d_provider)
    }

    pub fn from_name(name: &str) -> Result<Self, ImageTo3DProviderError> {
        match normalize_provider_name(name).as_str() {
            "meshy" | "meshy_image_to_3d" | "meshy-image-to-3d" => Ok(Self::Meshy),
            _ => Err(ImageTo3DProviderError::InvalidProvider {
                name: name.to_owned(),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meshy => "meshy",
        }
    }
}

impl Display for ImageTo3DProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageTo3DProviderError {
    InvalidProvider { name: String },
    SubmitFailed { message: String },
    PollFailed { message: String },
    DownloadFailed { message: String },
}

impl Display for ImageTo3DProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProvider { name } => {
                write!(formatter, "unsupported image-to-3d provider {name:?}")
            }
            Self::SubmitFailed { message } => {
                write!(formatter, "image-to-3d submit failed: {message}")
            }
            Self::PollFailed { message } => write!(formatter, "image-to-3d poll failed: {message}"),
            Self::DownloadFailed { message } => {
                write!(formatter, "image-to-3d download failed: {message}")
            }
        }
    }
}

impl Error for ImageTo3DProviderError {}

fn normalize_provider_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_selection_accepts_meshy_aliases() {
        assert_eq!(DEFAULT_IMAGE_TO_3D_PROVIDER, ImageTo3DProviderKind::Meshy);
        assert_eq!(
            ImageTo3DProviderKind::from_name(" meshy "),
            Ok(ImageTo3DProviderKind::Meshy)
        );
        assert_eq!(
            ImageTo3DProviderKind::from_name("meshy-image-to-3d"),
            Ok(ImageTo3DProviderKind::Meshy)
        );
    }

    #[test]
    fn provider_selection_rejects_unknown_provider() {
        assert_eq!(
            ImageTo3DProviderKind::from_name("local_fake"),
            Err(ImageTo3DProviderError::InvalidProvider {
                name: "local_fake".to_owned()
            })
        );
    }

    #[test]
    fn request_defaults_produce_textured_glb() {
        let request = ImageTo3DRequest::new("https://example.test/frame.png");

        assert_eq!(request.ai_model, "latest");
        assert_eq!(request.model_type, MeshModelType::Standard);
        assert!(request.should_texture);
        assert!(request.enable_pbr);
        assert!(request.should_remesh);
        assert_eq!(request.target_formats, ["glb"]);
    }
}
