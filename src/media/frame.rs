#![allow(dead_code)]

use std::{
    error::Error,
    ffi::OsString,
    fmt::{self, Display},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const DEFAULT_THUMBNAIL_SAMPLE_FRAMES: u16 = 80;
pub const DEFAULT_JPEG_QUALITY: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameExtractionOptions {
    pub ffmpeg_path: PathBuf,
    pub output_format: FrameImageFormat,
    pub thumbnail_sample_frames: u16,
    pub jpeg_quality: u8,
    pub overwrite: bool,
}

impl Default for FrameExtractionOptions {
    fn default() -> Self {
        Self {
            ffmpeg_path: PathBuf::from("ffmpeg"),
            output_format: FrameImageFormat::Jpeg,
            thumbnail_sample_frames: DEFAULT_THUMBNAIL_SAMPLE_FRAMES,
            jpeg_quality: DEFAULT_JPEG_QUALITY,
            overwrite: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameImageFormat {
    Jpeg,
    Png,
}

impl FrameImageFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFrame {
    pub path: PathBuf,
    pub content_type: &'static str,
    pub file_extension: &'static str,
}

pub fn extract_representative_frame(
    input_video: impl AsRef<Path>,
    output_image: impl AsRef<Path>,
) -> Result<ExtractedFrame, FrameExtractionError> {
    extract_representative_frame_with_options(
        input_video,
        output_image,
        &FrameExtractionOptions::default(),
    )
}

pub fn extract_representative_frame_with_options(
    input_video: impl AsRef<Path>,
    output_image: impl AsRef<Path>,
    options: &FrameExtractionOptions,
) -> Result<ExtractedFrame, FrameExtractionError> {
    let input_video = input_video.as_ref();
    let output_image = output_image.as_ref();
    validate_extraction(input_video, output_image, options)?;

    if let Some(parent) = output_image
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| FrameExtractionError::CreateOutputDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let args = build_ffmpeg_args(input_video, output_image, options)?;
    let output = Command::new(&options.ffmpeg_path)
        .args(&args)
        .output()
        .map_err(|source| FrameExtractionError::Launch {
            program: options.ffmpeg_path.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(FrameExtractionError::FfmpegFailed {
            status_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let metadata =
        fs::metadata(output_image).map_err(|source| FrameExtractionError::MissingOutput {
            path: output_image.to_path_buf(),
            source,
        })?;

    if metadata.len() == 0 {
        return Err(FrameExtractionError::EmptyOutput {
            path: output_image.to_path_buf(),
        });
    }

    Ok(ExtractedFrame {
        path: output_image.to_path_buf(),
        content_type: options.output_format.content_type(),
        file_extension: options.output_format.file_extension(),
    })
}

pub fn build_ffmpeg_args(
    input_video: &Path,
    output_image: &Path,
    options: &FrameExtractionOptions,
) -> Result<Vec<OsString>, FrameExtractionError> {
    validate_extraction(input_video, output_image, options)?;

    let thumbnail_filter = format!("thumbnail={}", options.thumbnail_sample_frames);
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("error"),
    ];

    if options.overwrite {
        args.push(OsString::from("-y"));
    } else {
        args.push(OsString::from("-n"));
    }

    args.extend([
        OsString::from("-i"),
        input_video.as_os_str().to_os_string(),
        OsString::from("-an"),
        OsString::from("-vf"),
        OsString::from(thumbnail_filter),
        OsString::from("-frames:v"),
        OsString::from("1"),
    ]);

    match options.output_format {
        FrameImageFormat::Jpeg => {
            args.extend([
                OsString::from("-q:v"),
                OsString::from(options.jpeg_quality.to_string()),
            ]);
        }
        FrameImageFormat::Png => {
            args.extend([OsString::from("-compression_level"), OsString::from("6")]);
        }
    }

    args.push(output_image.as_os_str().to_os_string());
    Ok(args)
}

fn validate_extraction(
    input_video: &Path,
    output_image: &Path,
    options: &FrameExtractionOptions,
) -> Result<(), FrameExtractionError> {
    if input_video.as_os_str().is_empty() {
        return Err(FrameExtractionError::InvalidInput {
            reason: "input video path is empty",
        });
    }

    if output_image.as_os_str().is_empty() {
        return Err(FrameExtractionError::InvalidInput {
            reason: "output image path is empty",
        });
    }

    if options.ffmpeg_path.as_os_str().is_empty() {
        return Err(FrameExtractionError::InvalidInput {
            reason: "ffmpeg path is empty",
        });
    }

    if options.thumbnail_sample_frames == 0 {
        return Err(FrameExtractionError::InvalidInput {
            reason: "thumbnail_sample_frames must be greater than zero",
        });
    }

    if options.jpeg_quality == 0 || options.jpeg_quality > 31 {
        return Err(FrameExtractionError::InvalidInput {
            reason: "jpeg_quality must be between 1 and 31",
        });
    }

    Ok(())
}

#[derive(Debug)]
pub enum FrameExtractionError {
    InvalidInput {
        reason: &'static str,
    },
    CreateOutputDir {
        path: PathBuf,
        source: std::io::Error,
    },
    Launch {
        program: PathBuf,
        source: std::io::Error,
    },
    FfmpegFailed {
        status_code: Option<i32>,
        stderr: String,
    },
    MissingOutput {
        path: PathBuf,
        source: std::io::Error,
    },
    EmptyOutput {
        path: PathBuf,
    },
}

impl Display for FrameExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { reason } => {
                write!(formatter, "invalid frame extraction input: {reason}")
            }
            Self::CreateOutputDir { path, source } => {
                write!(
                    formatter,
                    "failed to create output directory {}: {source}",
                    path.display()
                )
            }
            Self::Launch { program, source } => {
                write!(
                    formatter,
                    "failed to launch ffmpeg program {}: {source}",
                    program.display()
                )
            }
            Self::FfmpegFailed {
                status_code,
                stderr,
            } => {
                write!(
                    formatter,
                    "ffmpeg failed with status {}: {}",
                    status_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_owned()),
                    stderr
                )
            }
            Self::MissingOutput { path, source } => {
                write!(
                    formatter,
                    "ffmpeg did not create output frame {}: {source}",
                    path.display()
                )
            }
            Self::EmptyOutput { path } => {
                write!(
                    formatter,
                    "ffmpeg created an empty output frame {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for FrameExtractionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateOutputDir { source, .. }
            | Self::Launch { source, .. }
            | Self::MissingOutput { source, .. } => Some(source),
            Self::InvalidInput { .. } | Self::FfmpegFailed { .. } | Self::EmptyOutput { .. } => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_jpeg_args_use_thumbnail_filter_and_single_frame() {
        let args = build_ffmpeg_args(
            Path::new("clip.mp4"),
            Path::new("frame.jpg"),
            &FrameExtractionOptions::default(),
        )
        .expect("valid args");
        let args = args_to_strings(&args);

        assert_eq!(args[0], "-hide_banner");
        assert!(args.contains(&"-y".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-i", "clip.mp4"]));
        assert!(args.windows(2).any(|pair| pair == ["-vf", "thumbnail=80"]));
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "1"]));
        assert!(args.windows(2).any(|pair| pair == ["-q:v", "2"]));
        assert_eq!(args.last().map(String::as_str), Some("frame.jpg"));
    }

    #[test]
    fn png_args_use_png_compression() {
        let options = FrameExtractionOptions {
            output_format: FrameImageFormat::Png,
            overwrite: false,
            ..FrameExtractionOptions::default()
        };
        let args = build_ffmpeg_args(Path::new("clip.mp4"), Path::new("frame.png"), &options)
            .expect("valid args");
        let args = args_to_strings(&args);

        assert!(args.contains(&"-n".to_owned()));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-compression_level", "6"])
        );
        assert!(!args.contains(&"-q:v".to_owned()));
    }

    #[test]
    fn output_metadata_matches_format() {
        assert_eq!(FrameImageFormat::Jpeg.content_type(), "image/jpeg");
        assert_eq!(FrameImageFormat::Jpeg.file_extension(), "jpg");
        assert_eq!(FrameImageFormat::Png.content_type(), "image/png");
        assert_eq!(FrameImageFormat::Png.file_extension(), "png");
    }

    #[test]
    fn validation_rejects_zero_thumbnail_sample() {
        let options = FrameExtractionOptions {
            thumbnail_sample_frames: 0,
            ..FrameExtractionOptions::default()
        };
        let error = build_ffmpeg_args(Path::new("clip.mp4"), Path::new("frame.jpg"), &options)
            .expect_err("zero sample should fail");

        assert!(matches!(error, FrameExtractionError::InvalidInput { .. }));
    }

    fn args_to_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}
