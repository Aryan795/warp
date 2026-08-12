//! Client-side video-as-context support.
//!
//! LLM APIs we use don't accept native video attachments (only Google Gemini 2.5/3.x does today,
//! and wiring that up natively is a deliberate follow-up — see `FeatureFlag::VideoAsContext`).
//! Instead, this module extracts a small set of representative still frames from a locally
//! attached video so they can be sent through the existing image-as-context pipeline unchanged,
//! plus (optionally) an audio transcript that gets inserted as plain text.
//!
//! Frame extraction shells out to a system `ffmpeg` binary — the same approach already used
//! elsewhere in this codebase for computer-use screen recording thumbnails (see
//! `crates/computer_use/src/thumbnail.rs`). `ffmpeg` is not bundled with Warp:
//! - macOS: typically installed via Homebrew (`brew install ffmpeg`).
//! - Linux: available via the system package manager (e.g. `apt install ffmpeg`).
//! - Windows: available via winget/Chocolatey/scoop, or a manual download from ffmpeg.org.
//!
//! When `ffmpeg` isn't found on `PATH`, [`ffmpeg_available`] returns `false` and callers should
//! degrade gracefully (surface an actionable error instead of silently failing).
//!
//! Sampling strategy: naive fixed-fps sampling does badly on screen recordings (long static
//! stretches, missed cuts), so frames are chosen by scene-change detection (ffmpeg's `select`
//! filter) with a density floor — if a mostly-static video produces too few scene cuts, we top
//! up with evenly spaced frames so the model still sees how the video progresses over time.
//! Frame count is hard-capped well under the existing 20-images-per-query limit shared with
//! image-as-context.

use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

/// Video MIME types accepted for video-as-context. Kept to containers `ffmpeg` reliably demuxes
/// and that are common for screen recordings and short clips.
pub const SUPPORTED_VIDEO_MIME_TYPES: &[&str] = &[
    "video/mp4",
    "video/quicktime",
    "video/webm",
    "video/x-matroska",
];

/// Returns whether `mime_type` is a video type supported for video-as-context.
pub fn is_supported_video_mime_type(mime_type: &str) -> bool {
    SUPPORTED_VIDEO_MIME_TYPES.contains(&mime_type)
}

/// Hard cap on frames extracted from a single video. Keeps us comfortably under the existing
/// `MAX_IMAGE_COUNT_FOR_QUERY` (20) limit that image-as-context already enforces per query.
pub const MAX_VIDEO_FRAMES: usize = 16;

/// Density floor: if scene-change detection alone would yield fewer than this many frames (e.g.
/// a mostly-static screen recording with few visual cuts), evenly spaced frames top up the set
/// so the model still sees the video's progression over time.
pub const MIN_VIDEO_FRAMES: usize = 6;

/// Score threshold (0-1) for ffmpeg's `select='gt(scene,THRESHOLD)'` filter: a frame is treated
/// as a scene change when its histogram-difference score from the previous frame exceeds this.
const SCENE_CHANGE_THRESHOLD: f32 = 0.4;

/// Long-edge cap (px) for extracted frames: small enough to keep per-frame size well inside
/// image-as-context's own resize/size limits, in line with prior art for video-frame sampling.
const FRAME_MAX_EDGE_PX: u32 = 768;

/// Minimum spacing (seconds) enforced between two sampled timestamps, so density-floor top-up
/// frames don't land right next to an already-selected scene-change frame.
const MIN_FRAME_SPACING_SECS: f64 = 0.5;

/// A single extracted frame, encoded as JPEG bytes, in chronological order.
pub struct ExtractedFrame {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum VideoProcessingError {
    /// `ffmpeg` isn't on `PATH` (or isn't runnable).
    FfmpegUnavailable,
    /// `ffmpeg` ran but returned an error.
    Ffmpeg(String),
    /// Filesystem error reading back extracted output.
    Io(String),
    /// `ffmpeg` produced no usable frames for this video.
    NoFramesExtracted,
}

impl std::fmt::Display for VideoProcessingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoProcessingError::FfmpegUnavailable => write!(
                f,
                "ffmpeg isn't installed (or isn't on PATH). Install ffmpeg to attach videos as context."
            ),
            VideoProcessingError::Ffmpeg(reason) => write!(f, "ffmpeg error: {reason}"),
            VideoProcessingError::Io(reason) => write!(f, "{reason}"),
            VideoProcessingError::NoFramesExtracted => {
                write!(f, "couldn't extract any frames from this video")
            }
        }
    }
}

/// Returns whether a usable `ffmpeg` binary is available on `PATH`.
pub async fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// Extracts up to `max_frames` representative JPEG frames from `video_path` using scene-change
/// sampling with a density floor of `min_frames` (see module docs).
pub async fn extract_frames(
    video_path: &Path,
    max_frames: usize,
    min_frames: usize,
) -> Result<Vec<ExtractedFrame>, VideoProcessingError> {
    if !ffmpeg_available().await {
        return Err(VideoProcessingError::FfmpegUnavailable);
    }

    let duration_secs = probe_duration_secs(video_path).await;

    let mut timestamps = detect_scene_change_timestamps(video_path, max_frames).await;
    timestamps.truncate(max_frames);

    if timestamps.len() < min_frames
        && let Some(duration_secs) = duration_secs
        && duration_secs > 0.0
    {
        let wanted = min_frames.saturating_sub(timestamps.len());
        for candidate in evenly_spaced_timestamps(duration_secs, wanted) {
            if timestamps.len() >= max_frames {
                break;
            }
            let too_close = timestamps
                .iter()
                .any(|existing| (existing - candidate).abs() < MIN_FRAME_SPACING_SECS);
            if !too_close {
                timestamps.push(candidate);
            }
        }
    }

    if timestamps.is_empty() {
        // No scene cuts detected and duration couldn't be probed (or is degenerate); fall back
        // to a single frame at the start of the video so a genuinely short/static clip still
        // attaches something rather than failing outright.
        timestamps.push(0.0);
    }

    timestamps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    timestamps.truncate(max_frames);

    let temp_dir = tempfile::Builder::new()
        .prefix("warp-video-context-")
        .tempdir()
        .map_err(|e| VideoProcessingError::Io(format!("failed to create temp directory: {e}")))?;

    let mut frames = Vec::with_capacity(timestamps.len());
    for (index, timestamp) in timestamps.into_iter().enumerate() {
        let out_path = temp_dir.path().join(format!("frame-{index:03}.jpg"));
        if extract_single_frame(video_path, timestamp, &out_path)
            .await
            .is_err()
        {
            // Best-effort: skip frames ffmpeg couldn't seek to (e.g. right at EOF) rather than
            // failing the whole extraction.
            continue;
        }
        match tokio::fs::read(&out_path).await {
            Ok(data) => frames.push(ExtractedFrame { data }),
            Err(_) => continue,
        }
    }

    if frames.is_empty() {
        return Err(VideoProcessingError::NoFramesExtracted);
    }

    Ok(frames)
}

/// Extracts a single downscaled JPEG frame at `timestamp_secs` to `out_path`.
async fn extract_single_frame(
    video_path: &Path,
    timestamp_secs: f64,
    out_path: &Path,
) -> Result<(), VideoProcessingError> {
    let scale_filter = format!(
        "scale='min({FRAME_MAX_EDGE_PX},iw)':'min({FRAME_MAX_EDGE_PX},ih)':force_original_aspect_ratio=decrease"
    );
    let output = Command::new("ffmpeg")
        .arg("-y")
        .arg("-hide_banner")
        .arg("-ss")
        .arg(format!("{timestamp_secs:.3}"))
        .arg("-i")
        .arg(video_path)
        .arg("-frames:v")
        .arg("1")
        .arg("-vf")
        .arg(&scale_filter)
        .arg("-q:v")
        .arg("3")
        .arg(out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| VideoProcessingError::Ffmpeg(format!("failed to spawn ffmpeg: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(VideoProcessingError::Ffmpeg(stderr.trim_end().to_string()))
    }
}

/// Runs ffmpeg's `select` scene filter over the whole video and parses the `showinfo` filter's
/// stderr output for the presentation timestamp (`pts_time`) of each detected scene change.
async fn detect_scene_change_timestamps(video_path: &Path, max_frames: usize) -> Vec<f64> {
    let filter = format!("select='gt(scene,{SCENE_CHANGE_THRESHOLD})',showinfo");
    let Ok(output) = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(&filter)
        .arg("-frames:v")
        .arg((max_frames * 4).to_string())
        .arg("-f")
        .arg("null")
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
    else {
        return Vec::new();
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_showinfo_pts_times(&stderr)
}

/// Parses `pts_time:<float>` occurrences out of ffmpeg's `showinfo` filter stderr output.
fn parse_showinfo_pts_times(stderr: &str) -> Vec<f64> {
    let mut timestamps = Vec::new();
    for line in stderr.lines() {
        if !line.contains("Parsed_showinfo") {
            continue;
        }
        if let Some(rest) = line.split("pts_time:").nth(1) {
            let value: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(seconds) = value.parse::<f64>() {
                timestamps.push(seconds);
            }
        }
    }
    timestamps
}

/// Probes the video's duration (seconds) by parsing ffmpeg's own `Duration: HH:MM:SS.ss` stderr
/// banner, rather than depending on a separate `ffprobe` binary (some environments — including
/// certain sandboxes used for verification — ship `ffmpeg` without `ffprobe`). Returns `None` if
/// the duration couldn't be determined; callers treat that as "skip the density top-up" rather
/// than a hard failure, since scene-change frames may still be usable on their own.
async fn probe_duration_secs(video_path: &Path) -> Option<f64> {
    // No output is requested; ffmpeg always exits non-zero here (nothing was muxed), but it
    // still prints the input's `Duration: ...` line to stderr before that, which is all we need.
    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-i")
        .arg(video_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_ffmpeg_duration_secs(&stderr)
}

/// Parses a `Duration: HH:MM:SS.ss` line out of ffmpeg's stderr banner into total seconds.
fn parse_ffmpeg_duration_secs(stderr: &str) -> Option<f64> {
    let line = stderr
        .lines()
        .find(|line| line.trim_start().starts_with("Duration:"))?;
    let after_prefix = line.trim_start().strip_prefix("Duration:")?;
    let timestamp = after_prefix.split(',').next()?.trim();
    let mut parts = timestamp.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

/// Returns `count` timestamps evenly spaced across `(0, duration_secs)`, excluding the very
/// start/end so top-up frames land inside the video's visible content.
fn evenly_spaced_timestamps(duration_secs: f64, count: usize) -> Vec<f64> {
    if count == 0 {
        return Vec::new();
    }
    let step = duration_secs / (count + 1) as f64;
    (1..=count).map(|i| step * i as f64).collect()
}

/// Extracts the video's audio track as 16kHz mono PCM16 WAV bytes, suitable for the same
/// transcription endpoint used for voice input. Returns `None` if the video has no audio track
/// (rather than an error), since audio is optional for video-as-context.
pub async fn extract_audio_wav(video_path: &Path) -> Result<Option<Vec<u8>>, VideoProcessingError> {
    if !ffmpeg_available().await {
        return Err(VideoProcessingError::FfmpegUnavailable);
    }

    let temp_dir = tempfile::Builder::new()
        .prefix("warp-video-audio-")
        .tempdir()
        .map_err(|e| VideoProcessingError::Io(format!("failed to create temp directory: {e}")))?;
    let out_path = temp_dir.path().join("audio.wav");

    let output = Command::new("ffmpeg")
        .arg("-y")
        .arg("-hide_banner")
        .arg("-i")
        .arg(video_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg(&out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| VideoProcessingError::Ffmpeg(format!("failed to spawn ffmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // ffmpeg exits non-zero (with "Output file is empty") when the input has no audio
        // stream at all — treat that as "no audio" rather than a hard error.
        if stderr.contains("does not contain any stream") || stderr.contains("Output file is empty")
        {
            return Ok(None);
        }
        return Err(VideoProcessingError::Ffmpeg(stderr.trim_end().to_string()));
    }

    let data = tokio::fs::read(&out_path)
        .await
        .map_err(|e| VideoProcessingError::Io(format!("failed to read extracted audio: {e}")))?;
    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(data))
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;
