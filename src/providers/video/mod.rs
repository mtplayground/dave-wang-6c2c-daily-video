#![allow(dead_code)]

use std::{
    error::Error,
    fmt::{self, Display},
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{config::ProviderConfig, models::rotation::RotationAnimal};

pub const DEFAULT_VIDEO_DURATION_SECONDS: u16 = 8;
pub const DEFAULT_VIDEO_ASPECT_RATIO: &str = "9:16";
pub const DEFAULT_VIDEO_PROVIDER: VideoProviderKind = VideoProviderKind::GeminiVeo;

#[async_trait]
pub trait VideoProvider: Send + Sync {
    async fn submit_prompt(
        &self,
        request: VideoGenerationRequest,
    ) -> Result<VideoJob, VideoProviderError>;

    async fn poll_job(&self, job: &VideoJob) -> Result<VideoJobStatus, VideoProviderError>;

    async fn download_clip(&self, job: &VideoJob) -> Result<VideoClip, VideoProviderError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoGenerationRequest {
    pub prompt: VideoPrompt,
    pub poll_interval: Duration,
}

impl VideoGenerationRequest {
    pub fn new(prompt: VideoPrompt) -> Self {
        Self {
            prompt,
            poll_interval: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoPrompt {
    pub animal: RotationAnimal,
    pub text: String,
    pub duration_seconds: u16,
    pub aspect_ratio: String,
    pub negative_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoJob {
    pub provider: VideoProviderKind,
    pub provider_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoJobStatus {
    Pending,
    Running,
    Complete,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoClip {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub file_extension: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoProviderKind {
    GeminiVeo,
}

impl VideoProviderKind {
    pub fn from_config(config: &ProviderConfig) -> Result<Self, VideoProviderError> {
        Self::from_name(&config.video_provider)
    }

    pub fn from_name(name: &str) -> Result<Self, VideoProviderError> {
        match normalize_provider_name(name).as_str() {
            "gemini_veo" | "gemini-veo" | "veo" => Ok(Self::GeminiVeo),
            _ => Err(VideoProviderError::InvalidProvider {
                name: name.to_owned(),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GeminiVeo => "gemini_veo",
        }
    }
}

impl Display for VideoProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoProviderError {
    InvalidProvider { name: String },
    SubmitFailed { message: String },
    PollFailed { message: String },
    DownloadFailed { message: String },
}

impl Display for VideoProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProvider { name } => {
                write!(formatter, "unsupported video provider {name:?}")
            }
            Self::SubmitFailed { message } => write!(formatter, "video submit failed: {message}"),
            Self::PollFailed { message } => write!(formatter, "video poll failed: {message}"),
            Self::DownloadFailed { message } => {
                write!(formatter, "video download failed: {message}")
            }
        }
    }
}

impl Error for VideoProviderError {}

pub fn build_funny_video_prompt(animal: RotationAnimal) -> VideoPrompt {
    let scene = match animal {
        RotationAnimal::Dog => {
            "a cheerful dog in tiny rain boots dramatically slipping and recovering while chasing a bouncing tennis ball through a bright kitchen"
        }
        RotationAnimal::Cat => {
            "a curious cat wearing a small detective hat stealthily stalking a toy mouse, then acting surprised when it squeaks"
        }
        RotationAnimal::Rabbit => {
            "a fluffy rabbit as a clumsy magician pulling carrot after carrot from a tiny top hat on a sunny picnic blanket"
        }
        RotationAnimal::Pig => {
            "a joyful pig in a polka-dot apron triumphantly decorating cupcakes, accidentally dusting itself with flour like a chef"
        }
        RotationAnimal::Chicken => {
            "a confident chicken leading a backyard marching band of wind-up toys, pausing for a proud little dance"
        }
    };

    VideoPrompt {
        animal,
        text: format!(
            "Create a funny, family-friendly short vertical video of {scene}. Keep the animal as the clear main subject, use lively physical comedy, warm natural lighting, smooth camera motion, and a complete gag with a clear beginning, middle, and end. No dialogue, captions, logos, watermarks, or on-screen text."
        ),
        duration_seconds: DEFAULT_VIDEO_DURATION_SECONDS,
        aspect_ratio: DEFAULT_VIDEO_ASPECT_RATIO.to_owned(),
        negative_prompt: "scary, violent, realistic injury, distorted anatomy, extra limbs, text overlays, subtitles, logos, watermarks, brand names".to_owned(),
    }
}

fn normalize_provider_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_is_gemini_veo() {
        assert_eq!(DEFAULT_VIDEO_PROVIDER, VideoProviderKind::GeminiVeo);
        assert_eq!(
            VideoProviderKind::from_name(" gemini_veo "),
            Ok(VideoProviderKind::GeminiVeo)
        );
        assert_eq!(
            VideoProviderKind::from_name("VEO"),
            Ok(VideoProviderKind::GeminiVeo)
        );
    }

    #[test]
    fn unknown_provider_is_rejected() {
        assert_eq!(
            VideoProviderKind::from_name("local_fake"),
            Err(VideoProviderError::InvalidProvider {
                name: "local_fake".to_owned()
            })
        );
    }

    #[test]
    fn prompt_builder_sets_generation_defaults() {
        let prompt = build_funny_video_prompt(RotationAnimal::Dog);

        assert_eq!(prompt.animal, RotationAnimal::Dog);
        assert_eq!(prompt.duration_seconds, DEFAULT_VIDEO_DURATION_SECONDS);
        assert_eq!(prompt.aspect_ratio, DEFAULT_VIDEO_ASPECT_RATIO);
        assert!(prompt.text.contains("vertical video"));
        assert!(prompt.text.contains("No dialogue"));
        assert!(prompt.negative_prompt.contains("watermarks"));
    }

    #[test]
    fn prompt_builder_mentions_each_rotation_animal() {
        for animal in RotationAnimal::CYCLE {
            let prompt = build_funny_video_prompt(animal);
            let text = prompt.text.to_ascii_lowercase();

            assert!(
                text.contains(animal_name(animal)),
                "prompt for {animal:?} should mention its animal name: {}",
                prompt.text
            );
        }
    }

    #[test]
    fn animal_prompts_are_distinct() {
        let prompts: Vec<_> = RotationAnimal::CYCLE
            .into_iter()
            .map(|animal| build_funny_video_prompt(animal).text)
            .collect();

        for (index, prompt) in prompts.iter().enumerate() {
            assert!(
                !prompts.iter().skip(index + 1).any(|other| other == prompt),
                "animal prompts should not repeat"
            );
        }
    }

    fn animal_name(animal: RotationAnimal) -> &'static str {
        match animal {
            RotationAnimal::Dog => "dog",
            RotationAnimal::Cat => "cat",
            RotationAnimal::Rabbit => "rabbit",
            RotationAnimal::Pig => "pig",
            RotationAnimal::Chicken => "chicken",
        }
    }
}
