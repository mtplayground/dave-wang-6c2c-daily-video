use std::{ffi::OsString, path::Path};

use dave_wang_6c2c_daily_video::media::assemble::{
    AssemblyError, AssemblyOptions, DEFAULT_ASSEMBLY_FPS, DEFAULT_ASSEMBLY_HEIGHT,
    DEFAULT_ASSEMBLY_WIDTH, DEFAULT_AUDIO_BITRATE, DEFAULT_VIDEO_CRF, FinalVideo,
    build_ffmpeg_assembly_args, concat_filter,
};

#[test]
fn assembly_args_normalize_concat_and_encode_final_mp4() {
    let options = AssemblyOptions::default();
    let args = build_ffmpeg_assembly_args(
        Path::new("funny.mp4"),
        Path::new("reveal.mp4"),
        Path::new("final.mp4"),
        &options,
    )
    .expect("valid args");
    let args = args_to_strings(&args);

    assert_eq!(args[0], "-hide_banner");
    assert!(args.contains(&"-y".to_owned()));
    assert!(args.windows(2).any(|pair| pair == ["-i", "funny.mp4"]));
    assert!(args.windows(2).any(|pair| pair == ["-i", "reveal.mp4"]));
    assert!(args.windows(2).any(|pair| pair == ["-map", "[v]"]));
    assert!(args.windows(2).any(|pair| pair == ["-map", "0:a?"]));
    assert!(args.windows(2).any(|pair| pair == ["-c:v", "libx264"]));
    assert!(args.windows(2).any(|pair| pair == ["-crf", "18"]));
    assert!(args.windows(2).any(|pair| pair == ["-pix_fmt", "yuv420p"]));
    assert!(args.windows(2).any(|pair| pair == ["-c:a", "aac"]));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-b:a", DEFAULT_AUDIO_BITRATE])
    );
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-movflags", "+faststart"])
    );
    assert_eq!(args.last().map(String::as_str), Some("final.mp4"));
}

#[test]
fn concat_filter_scales_pads_fps_and_concats_video_only() {
    let filter = concat_filter(&AssemblyOptions::default());

    assert!(filter.contains(&format!(
        "scale={}:{}:force_original_aspect_ratio=decrease",
        DEFAULT_ASSEMBLY_WIDTH, DEFAULT_ASSEMBLY_HEIGHT
    )));
    assert!(filter.contains("pad=1080:1920:(ow-iw)/2:(oh-ih)/2:color=black"));
    assert!(filter.contains(&format!("fps={DEFAULT_ASSEMBLY_FPS}")));
    assert!(filter.contains("setsar=1"));
    assert!(filter.contains("format=yuv420p"));
    assert!(filter.ends_with("[v0][v1]concat=n=2:v=1:a=0[v]"));
}

#[test]
fn no_overwrite_uses_ffmpeg_no_clobber_flag() {
    let options = AssemblyOptions {
        overwrite: false,
        ..AssemblyOptions::default()
    };
    let args = build_ffmpeg_assembly_args(
        Path::new("funny.mp4"),
        Path::new("reveal.mp4"),
        Path::new("final.mp4"),
        &options,
    )
    .expect("valid args");
    let args = args_to_strings(&args);

    assert!(args.contains(&"-n".to_owned()));
    assert!(!args.contains(&"-y".to_owned()));
}

#[test]
fn validation_rejects_invalid_crf() {
    let options = AssemblyOptions {
        video_crf: 52,
        ..AssemblyOptions::default()
    };
    let error = build_ffmpeg_assembly_args(
        Path::new("funny.mp4"),
        Path::new("reveal.mp4"),
        Path::new("final.mp4"),
        &options,
    )
    .expect_err("invalid crf should fail");

    assert!(matches!(error, AssemblyError::InvalidInput { .. }));
}

#[test]
fn validation_rejects_zero_fps() {
    let options = AssemblyOptions {
        fps: 0,
        ..AssemblyOptions::default()
    };
    let error = build_ffmpeg_assembly_args(
        Path::new("funny.mp4"),
        Path::new("reveal.mp4"),
        Path::new("final.mp4"),
        &options,
    )
    .expect_err("zero fps should fail");

    assert!(matches!(error, AssemblyError::InvalidInput { .. }));
}

#[test]
fn final_video_metadata_is_mp4() {
    let video = FinalVideo {
        path: "final.mp4".into(),
        content_type: "video/mp4",
        file_extension: "mp4",
    };

    assert_eq!(video.content_type, "video/mp4");
    assert_eq!(video.file_extension, "mp4");
    assert_eq!(DEFAULT_VIDEO_CRF, 18);
}

fn args_to_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}
