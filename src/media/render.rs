#![allow(dead_code)]

use std::{
    error::Error,
    ffi::OsString,
    fmt::{self, Display},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const DEFAULT_RENDER_WIDTH: u16 = 1080;
pub const DEFAULT_RENDER_HEIGHT: u16 = 1920;
pub const DEFAULT_RENDER_FPS: u16 = 30;
pub const DEFAULT_RENDER_DURATION_SECONDS: u16 = 4;
pub const DEFAULT_RENDER_SAMPLES: u16 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurntableRenderOptions {
    pub blender_path: PathBuf,
    pub width: u16,
    pub height: u16,
    pub fps: u16,
    pub duration_seconds: u16,
    pub samples: u16,
    pub overwrite: bool,
}

impl Default for TurntableRenderOptions {
    fn default() -> Self {
        Self {
            blender_path: PathBuf::from("blender"),
            width: DEFAULT_RENDER_WIDTH,
            height: DEFAULT_RENDER_HEIGHT,
            fps: DEFAULT_RENDER_FPS,
            duration_seconds: DEFAULT_RENDER_DURATION_SECONDS,
            samples: DEFAULT_RENDER_SAMPLES,
            overwrite: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedClip {
    pub path: PathBuf,
    pub content_type: &'static str,
    pub file_extension: &'static str,
    pub duration_seconds: u16,
    pub fps: u16,
}

pub fn render_glb_turntable(
    input_glb: impl AsRef<Path>,
    output_clip: impl AsRef<Path>,
) -> Result<RenderedClip, RenderError> {
    render_glb_turntable_with_options(input_glb, output_clip, &TurntableRenderOptions::default())
}

pub fn render_glb_turntable_with_options(
    input_glb: impl AsRef<Path>,
    output_clip: impl AsRef<Path>,
    options: &TurntableRenderOptions,
) -> Result<RenderedClip, RenderError> {
    let input_glb = input_glb.as_ref();
    let output_clip = output_clip.as_ref();
    validate_render(input_glb, output_clip, options)?;

    if let Some(parent) = output_clip
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| RenderError::CreateOutputDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let args = build_blender_args(input_glb, output_clip, options)?;
    let output = Command::new(&options.blender_path)
        .args(&args)
        .output()
        .map_err(|source| RenderError::Launch {
            program: options.blender_path.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(RenderError::BlenderFailed {
            status_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let metadata = fs::metadata(output_clip).map_err(|source| RenderError::MissingOutput {
        path: output_clip.to_path_buf(),
        source,
    })?;

    if metadata.len() == 0 {
        return Err(RenderError::EmptyOutput {
            path: output_clip.to_path_buf(),
        });
    }

    Ok(RenderedClip {
        path: output_clip.to_path_buf(),
        content_type: "video/mp4",
        file_extension: "mp4",
        duration_seconds: options.duration_seconds,
        fps: options.fps,
    })
}

pub fn build_blender_args(
    input_glb: &Path,
    output_clip: &Path,
    options: &TurntableRenderOptions,
) -> Result<Vec<OsString>, RenderError> {
    validate_render(input_glb, output_clip, options)?;

    let mut args = vec![
        OsString::from("--background"),
        OsString::from("--factory-startup"),
        OsString::from("--python-expr"),
        OsString::from(BLENDER_TURNTABLE_SCRIPT),
        OsString::from("--"),
        input_glb.as_os_str().to_os_string(),
        output_clip.as_os_str().to_os_string(),
        OsString::from(options.width.to_string()),
        OsString::from(options.height.to_string()),
        OsString::from(options.fps.to_string()),
        OsString::from(options.duration_seconds.to_string()),
        OsString::from(options.samples.to_string()),
    ];

    if options.overwrite {
        args.push(OsString::from("overwrite"));
    } else {
        args.push(OsString::from("no-overwrite"));
    }

    Ok(args)
}

fn validate_render(
    input_glb: &Path,
    output_clip: &Path,
    options: &TurntableRenderOptions,
) -> Result<(), RenderError> {
    if input_glb.as_os_str().is_empty() {
        return Err(RenderError::InvalidInput {
            reason: "input GLB path is empty",
        });
    }

    if output_clip.as_os_str().is_empty() {
        return Err(RenderError::InvalidInput {
            reason: "output clip path is empty",
        });
    }

    if options.blender_path.as_os_str().is_empty() {
        return Err(RenderError::InvalidInput {
            reason: "blender path is empty",
        });
    }

    if options.width == 0 || options.height == 0 {
        return Err(RenderError::InvalidInput {
            reason: "render width and height must be greater than zero",
        });
    }

    if options.fps == 0 {
        return Err(RenderError::InvalidInput {
            reason: "fps must be greater than zero",
        });
    }

    if options.duration_seconds == 0 {
        return Err(RenderError::InvalidInput {
            reason: "duration_seconds must be greater than zero",
        });
    }

    if options.samples == 0 {
        return Err(RenderError::InvalidInput {
            reason: "samples must be greater than zero",
        });
    }

    Ok(())
}

#[derive(Debug)]
pub enum RenderError {
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
    BlenderFailed {
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

impl Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { reason } => write!(formatter, "invalid render input: {reason}"),
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
                    "failed to launch Blender program {}: {source}",
                    program.display()
                )
            }
            Self::BlenderFailed {
                status_code,
                stderr,
            } => {
                write!(
                    formatter,
                    "Blender failed with status {}: {}",
                    status_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_owned()),
                    stderr
                )
            }
            Self::MissingOutput { path, source } => {
                write!(
                    formatter,
                    "Blender did not create output clip {}: {source}",
                    path.display()
                )
            }
            Self::EmptyOutput { path } => {
                write!(
                    formatter,
                    "Blender created an empty output clip {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateOutputDir { source, .. }
            | Self::Launch { source, .. }
            | Self::MissingOutput { source, .. } => Some(source),
            Self::InvalidInput { .. } | Self::BlenderFailed { .. } | Self::EmptyOutput { .. } => {
                None
            }
        }
    }
}

pub const BLENDER_TURNTABLE_SCRIPT: &str = r#"
import math
import sys
import bpy
from mathutils import Vector

def fail(message):
    print(message, file=sys.stderr)
    sys.exit(2)

argv = sys.argv
if "--" not in argv:
    fail("missing render arguments")

args = argv[argv.index("--") + 1:]
if len(args) != 8:
    fail("expected input_glb output_mp4 width height fps duration samples overwrite_mode")

input_glb, output_mp4, width, height, fps, duration, samples, overwrite_mode = args
width = int(width)
height = int(height)
fps = int(fps)
duration = int(duration)
samples = int(samples)
if overwrite_mode != "overwrite":
    import os
    if os.path.exists(output_mp4):
        fail("output exists and overwrite is disabled")

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete()
bpy.ops.import_scene.gltf(filepath=input_glb)

mesh_objects = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
if not mesh_objects:
    fail("GLB did not contain renderable mesh objects")

bpy.context.view_layer.update()
min_corner = Vector((float("inf"), float("inf"), float("inf")))
max_corner = Vector((float("-inf"), float("-inf"), float("-inf")))
for obj in mesh_objects:
    for corner in obj.bound_box:
        world_corner = obj.matrix_world @ Vector(corner)
        min_corner.x = min(min_corner.x, world_corner.x)
        min_corner.y = min(min_corner.y, world_corner.y)
        min_corner.z = min(min_corner.z, world_corner.z)
        max_corner.x = max(max_corner.x, world_corner.x)
        max_corner.y = max(max_corner.y, world_corner.y)
        max_corner.z = max(max_corner.z, world_corner.z)

center = (min_corner + max_corner) * 0.5
extent = max(max_corner.x - min_corner.x, max_corner.y - min_corner.y, max_corner.z - min_corner.z)
if extent <= 0:
    extent = 1.0

turntable = bpy.data.objects.new("turntable_root", None)
bpy.context.collection.objects.link(turntable)
for obj in mesh_objects:
    obj.location = obj.location - center
    obj.parent = turntable
turntable.scale = (2.4 / extent, 2.4 / extent, 2.4 / extent)

bpy.ops.mesh.primitive_cylinder_add(vertices=96, radius=1.65, depth=0.08, location=(0, 0, -1.25))
platform = bpy.context.object
platform.name = "matte_print_bed"
mat = bpy.data.materials.new("warm_matte_print_bed")
mat.diffuse_color = (0.68, 0.64, 0.58, 1.0)
platform.data.materials.append(mat)

bpy.ops.mesh.primitive_cylinder_add(vertices=32, radius=0.08, depth=0.55, location=(0, -1.35, 1.55), rotation=(math.radians(90), 0, 0))
nozzle = bpy.context.object
nozzle.name = "print_nozzle_hint"
nozzle_mat = bpy.data.materials.new("dark_print_nozzle")
nozzle_mat.diffuse_color = (0.06, 0.07, 0.08, 1.0)
nozzle.data.materials.append(nozzle_mat)

bpy.ops.object.light_add(type="AREA", location=(0, -4, 4))
key_light = bpy.context.object
key_light.name = "large_softbox"
key_light.data.energy = 500
key_light.data.size = 4

bpy.ops.object.camera_add(location=(0, -5.1, 1.6), rotation=(math.radians(73), 0, 0))
camera = bpy.context.object
camera.data.lens = 45
bpy.context.scene.camera = camera

scene = bpy.context.scene
scene.render.engine = "CYCLES"
scene.cycles.samples = samples
scene.cycles.use_denoising = True
scene.render.resolution_x = width
scene.render.resolution_y = height
scene.render.fps = fps
scene.frame_start = 1
scene.frame_end = fps * duration
scene.world.color = (0.78, 0.80, 0.84)
scene.render.film_transparent = False

turntable.rotation_euler = (0, 0, 0)
turntable.keyframe_insert(data_path="rotation_euler", frame=1)
turntable.rotation_euler = (0, 0, math.radians(360))
turntable.keyframe_insert(data_path="rotation_euler", frame=scene.frame_end)

nozzle.location.z = 2.4
nozzle.keyframe_insert(data_path="location", frame=1)
nozzle.location.z = 1.55
nozzle.keyframe_insert(data_path="location", frame=max(2, int(scene.frame_end * 0.35)))
nozzle.location.z = 2.4
nozzle.keyframe_insert(data_path="location", frame=scene.frame_end)

for obj in [turntable, nozzle]:
    if obj.animation_data and obj.animation_data.action:
        for fcurve in obj.animation_data.action.fcurves:
            for keyframe in fcurve.keyframe_points:
                keyframe.interpolation = "BEZIER"

scene.render.image_settings.file_format = "FFMPEG"
scene.render.ffmpeg.format = "MPEG4"
scene.render.ffmpeg.codec = "H264"
scene.render.ffmpeg.constant_rate_factor = "HIGH"
scene.render.ffmpeg.ffmpeg_preset = "GOOD"
scene.render.filepath = output_mp4
bpy.ops.render.render(animation=True)
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blender_args_include_headless_script_and_render_parameters() {
        let args = build_blender_args(
            Path::new("model.glb"),
            Path::new("reveal.mp4"),
            &TurntableRenderOptions::default(),
        )
        .expect("valid args");
        let args = args_to_strings(&args);

        assert_eq!(args[0], "--background");
        assert!(args.contains(&"--factory-startup".to_owned()));
        assert!(args.contains(&"--python-expr".to_owned()));
        assert!(args.contains(&"model.glb".to_owned()));
        assert!(args.contains(&"reveal.mp4".to_owned()));
        assert!(args.contains(&DEFAULT_RENDER_WIDTH.to_string()));
        assert!(args.contains(&DEFAULT_RENDER_HEIGHT.to_string()));
        assert!(args.contains(&DEFAULT_RENDER_FPS.to_string()));
        assert!(args.contains(&DEFAULT_RENDER_DURATION_SECONDS.to_string()));
        assert_eq!(args.last().map(String::as_str), Some("overwrite"));
    }

    #[test]
    fn blender_script_imports_glb_and_renders_h264_turntable() {
        assert!(BLENDER_TURNTABLE_SCRIPT.contains("bpy.ops.import_scene.gltf"));
        assert!(BLENDER_TURNTABLE_SCRIPT.contains("turntable.rotation_euler"));
        assert!(BLENDER_TURNTABLE_SCRIPT.contains("H264"));
        assert!(BLENDER_TURNTABLE_SCRIPT.contains("print_nozzle_hint"));
    }

    #[test]
    fn validation_rejects_zero_dimensions() {
        let options = TurntableRenderOptions {
            width: 0,
            ..TurntableRenderOptions::default()
        };
        let error = build_blender_args(Path::new("model.glb"), Path::new("reveal.mp4"), &options)
            .expect_err("zero width should fail");

        assert!(matches!(error, RenderError::InvalidInput { .. }));
    }

    #[test]
    fn rendered_clip_metadata_is_mp4() {
        let clip = RenderedClip {
            path: PathBuf::from("reveal.mp4"),
            content_type: "video/mp4",
            file_extension: "mp4",
            duration_seconds: DEFAULT_RENDER_DURATION_SECONDS,
            fps: DEFAULT_RENDER_FPS,
        };

        assert_eq!(clip.content_type, "video/mp4");
        assert_eq!(clip.file_extension, "mp4");
    }

    fn args_to_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}
