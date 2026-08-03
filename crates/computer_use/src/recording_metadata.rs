use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::RecordingError;

/// Shortest media timeline a finalized recording may report and still be
/// treated as playable. A capture whose container duration is written near zero
/// plays back as an empty video even when every frame is present, so anything
/// under roughly a couple of frames is treated as unplayable.
const MIN_PLAYABLE_DURATION: Duration = Duration::from_millis(100);

/// Resolves the file that may be published for `input`: the path whose
/// container reports a playable timeline, together with that duration.
///
/// A mis-written container duration is recoverable without re-encoding, so a
/// single stream copy is attempted before giving up. The wall-clock capture
/// duration is deliberately never used as a fallback here: it cannot
/// distinguish a playable file from an unplayable one, which is exactly how a
/// zero-length recording reaches an upload.
pub(super) async fn playable_video(input: &Path) -> Result<(PathBuf, Duration), RecordingError> {
    let probe_failure = match video_duration(input).await {
        Ok(duration) if duration >= MIN_PLAYABLE_DURATION => {
            return Ok((input.to_path_buf(), duration));
        }
        Ok(duration) => format!("finalized video reported a {duration:?} timeline"),
        Err(error) => error.to_string(),
    };

    let remuxed = remux(input)
        .await
        .map_err(|error| RecordingError::Finalize {
            reason: format!("{probe_failure}; stream-copy repair failed: {error}"),
        })?;
    match video_duration(&remuxed).await {
        Ok(duration) if duration >= MIN_PLAYABLE_DURATION => Ok((remuxed, duration)),
        repaired => {
            let _ = std::fs::remove_file(&remuxed);
            let repaired = match repaired {
                Ok(duration) => format!("a {duration:?} timeline"),
                Err(error) => error.to_string(),
            };
            Err(RecordingError::Finalize {
                reason: format!("{probe_failure}; stream-copy repair still reported {repaired}"),
            })
        }
    }
}

/// Rewrites `input`'s container without re-encoding, rebuilding the media
/// timeline from the elementary stream's own timestamps.
async fn remux(input: &Path) -> Result<PathBuf, RecordingError> {
    let output_path = input.with_extension("remux.mp4");
    let output = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-i"])
        .arg(input)
        .args(["-c", "copy"])
        .args(["-movflags", "+faststart"])
        .arg(&output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to run ffmpeg for stream copy: {error}"),
        })?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&output_path);
        let detail = ffmpeg_error_tail(&String::from_utf8_lossy(&output.stderr));
        return Err(RecordingError::Finalize {
            reason: format!(
                "ffmpeg stream copy exited with status {}{detail}",
                output.status
            ),
        });
    }
    Ok(output_path)
}

/// Returns a short, parenthesized tail of ffmpeg's stderr for diagnostics.
pub(crate) fn ffmpeg_error_tail(log: &str) -> String {
    let lines: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(3);
    let tail = lines[start..].join(" ");
    if tail.is_empty() {
        String::new()
    } else {
        format!(" ({tail})")
    }
}

pub(super) async fn video_duration(input: &Path) -> Result<Duration, RecordingError> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to inspect finalized video: {error}"),
        })?;

    parse_duration(&String::from_utf8_lossy(&output.stderr)).ok_or_else(|| {
        RecordingError::Finalize {
            reason: "ffmpeg did not report a valid finalized video duration".to_string(),
        }
    })
}

fn parse_duration(stderr: &str) -> Option<Duration> {
    let timestamp = stderr.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Duration:")
            .and_then(|value| value.split(',').next())
            .map(str::trim)
    })?;
    let mut components = timestamp.split(':');
    let hours = components.next()?.parse::<u64>().ok()?;
    let minutes = components.next()?.parse::<u64>().ok()?;
    let seconds = components.next()?;
    if components.next().is_some() || minutes >= 60 {
        return None;
    }

    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let seconds = seconds.parse::<u64>().ok()?;
    if seconds >= 60 || fraction.len() > 9 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let nanos = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().ok()? * 10_u32.pow(9 - fraction.len() as u32)
    };
    let seconds = hours
        .checked_mul(60 * 60)?
        .checked_add(minutes * 60)?
        .checked_add(seconds)?;
    Some(Duration::new(seconds, nanos))
}

#[cfg(test)]
#[path = "recording_metadata_tests.rs"]
mod tests;
