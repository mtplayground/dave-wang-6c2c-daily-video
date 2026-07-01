#![allow(dead_code)]

use std::{
    error::Error,
    ffi::OsString,
    fmt::{self, Display},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const DEFAULT_ASSEMBLY_WIDTH: u16 = 1080;
pub const DEFAULT_ASSEMBLY_HEIGHT: u16 = 1920;
pub const DEFAULT_ASSEMBLY_FPS: u16 = 30;
pub const DEFAULT_VIDEO_CRF: u8 = 18;
pub const DEFAULT_AUDIO_BITRATE: &str = "160k";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyOptions {
    pub ffmpeg_path: PathBuf,
    pub width: u16,
    pub height: u16,
    pub fps: u16,
    pub video_crf: u8,
    pub audio_bitrate: String,
    pub overwrite: bool,
}

impl Default for AssemblyOptions {
    fn default() -> Self {
        Self {
            ffmpeg_path: PathBuf::from("ffmpeg"),
            width: DEFAULT_ASSEMBLY_WIDTH,
            height: DEFAULT_ASSEMBLY_HEIGHT,
            fps: DEFAULT_ASSEMBLY_FPS,
            video_crf: DEFAULT_VIDEO_CRF,
            audio_bitrate: DEFAULT_AUDIO_BITRATE.to_owned(),
            overwrite: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalVideo {
    pub path: PathBuf,
    pub content_type: &'static str,
    pub file_extension: &'static str,
}

pub fn assemble_final_mp4(
    funny_video: impl AsRef<Path>,
    reveal_clip: impl AsRef<Path>,
    output_mp4: impl AsRef<Path>,
) -> Result<FinalVideo, AssemblyError> {
    assemble_final_mp4_with_options(
        funny_video,
        reveal_clip,
        output_mp4,
        &AssemblyOptions::default(),
    )
}

pub fn assemble_final_mp4_with_options(
    funny_video: impl AsRef<Path>,
    reveal_clip: impl AsRef<Path>,
    output_mp4: impl AsRef<Path>,
    options: &AssemblyOptions,
) -> Result<FinalVideo, AssemblyError> {
    let funny_video = funny_video.as_ref();
    let reveal_clip = reveal_clip.as_ref();
    let output_mp4 = output_mp4.as_ref();
    validate_assembly(funny_video, reveal_clip, output_mp4, options)?;

    if let Some(parent) = output_mp4
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| AssemblyError::CreateOutputDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let args = build_ffmpeg_assembly_args(funny_video, reveal_clip, output_mp4, options)?;
    let output = Command::new(&options.ffmpeg_path)
        .args(&args)
        .output()
        .map_err(|source| AssemblyError::Launch {
            program: options.ffmpeg_path.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(AssemblyError::FfmpegFailed {
            status_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let metadata = fs::metadata(output_mp4).map_err(|source| AssemblyError::MissingOutput {
        path: output_mp4.to_path_buf(),
        source,
    })?;

    if metadata.len() == 0 {
        return Err(AssemblyError::EmptyOutput {
            path: output_mp4.to_path_buf(),
        });
    }

    Ok(FinalVideo {
        path: output_mp4.to_path_buf(),
        content_type: "video/mp4",
        file_extension: "mp4",
    })
}

pub fn build_ffmpeg_assembly_args(
    funny_video: &Path,
    reveal_clip: &Path,
    output_mp4: &Path,
    options: &AssemblyOptions,
) -> Result<Vec<OsString>, AssemblyError> {
    validate_assembly(funny_video, reveal_clip, output_mp4, options)?;

    let filter_complex = concat_filter(options);
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
        funny_video.as_os_str().to_os_string(),
        OsString::from("-i"),
        reveal_clip.as_os_str().to_os_string(),
        OsString::from("-filter_complex"),
        OsString::from(filter_complex),
        OsString::from("-map"),
        OsString::from("[v]"),
        OsString::from("-map"),
        OsString::from("0:a?"),
        OsString::from("-c:v"),
        OsString::from("libx264"),
        OsString::from("-preset"),
        OsString::from("medium"),
        OsString::from("-crf"),
        OsString::from(options.video_crf.to_string()),
        OsString::from("-pix_fmt"),
        OsString::from("yuv420p"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        OsString::from("-b:a"),
        OsString::from(options.audio_bitrate.clone()),
        OsString::from("-movflags"),
        OsString::from("+faststart"),
        OsString::from("-max_muxing_queue_size"),
        OsString::from("1024"),
        output_mp4.as_os_str().to_os_string(),
    ]);

    Ok(args)
}

pub fn concat_filter(options: &AssemblyOptions) -> String {
    let normalize = |input: usize, output: &str| {
        format!(
            "[{input}:v]scale={width}:{height}:force_original_aspect_ratio=decrease,\
pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,\
fps={fps},setsar=1,format=yuv420p[{output}]",
            width = options.width,
            height = options.height,
            fps = options.fps
        )
    };

    format!(
        "{};{};[v0][v1]concat=n=2:v=1:a=0[v]",
        normalize(0, "v0"),
        normalize(1, "v1")
    )
}

fn validate_assembly(
    funny_video: &Path,
    reveal_clip: &Path,
    output_mp4: &Path,
    options: &AssemblyOptions,
) -> Result<(), AssemblyError> {
    if funny_video.as_os_str().is_empty() {
        return Err(AssemblyError::InvalidInput {
            reason: "funny video path is empty",
        });
    }

    if reveal_clip.as_os_str().is_empty() {
        return Err(AssemblyError::InvalidInput {
            reason: "reveal clip path is empty",
        });
    }

    if output_mp4.as_os_str().is_empty() {
        return Err(AssemblyError::InvalidInput {
            reason: "output MP4 path is empty",
        });
    }

    if options.ffmpeg_path.as_os_str().is_empty() {
        return Err(AssemblyError::InvalidInput {
            reason: "ffmpeg path is empty",
        });
    }

    if options.width == 0 || options.height == 0 {
        return Err(AssemblyError::InvalidInput {
            reason: "assembly width and height must be greater than zero",
        });
    }

    if options.fps == 0 {
        return Err(AssemblyError::InvalidInput {
            reason: "fps must be greater than zero",
        });
    }

    if options.video_crf > 51 {
        return Err(AssemblyError::InvalidInput {
            reason: "video_crf must be between 0 and 51",
        });
    }

    if options.audio_bitrate.trim().is_empty() {
        return Err(AssemblyError::InvalidInput {
            reason: "audio_bitrate is empty",
        });
    }

    Ok(())
}

#[derive(Debug)]
pub enum AssemblyError {
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

impl Display for AssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { reason } => write!(formatter, "invalid assembly input: {reason}"),
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
                    "ffmpeg did not create final MP4 {}: {source}",
                    path.display()
                )
            }
            Self::EmptyOutput { path } => {
                write!(
                    formatter,
                    "ffmpeg created an empty final MP4 {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for AssemblyError {
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
