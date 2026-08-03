use std::time::Duration;

use tokio::process::Command;

use super::*;

#[test]
fn parses_ffmpeg_container_duration() {
    let stderr = "  Duration: 01:02:03.456789, start: 0.000000, bitrate: 64 kb/s";

    assert_eq!(
        parse_duration(stderr),
        Some(Duration::new(60 * 60 + 2 * 60 + 3, 456_789_000))
    );
}

#[test]
fn rejects_missing_or_invalid_duration() {
    for stderr in [
        "",
        "Duration: N/A, start: 0.000000",
        "Duration: 00:60:00.00, start: 0.000000",
        "Duration: 00:00:60.00, start: 0.000000",
    ] {
        assert_eq!(parse_duration(stderr), None);
    }
}

async fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .is_ok_and(|output| output.status.success())
}

#[tokio::test]
async fn probes_duration_after_timestamp_rescaling() {
    if !ffmpeg_available().await {
        return;
    }

    let path = std::env::temp_dir().join(format!(
        "warp-duration-probe-test-{}.mp4",
        uuid::Uuid::new_v4()
    ));
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=16x16:rate=10:duration=4",
            "-vf",
            "setpts=0.25*PTS",
            "-an",
            "-r",
            "10",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .output()
        .await
        .expect("run ffmpeg");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let duration = video_duration(&path).await.expect("probe duration");
    let _ = std::fs::remove_file(path);

    assert!(
        (Duration::from_millis(800)..=Duration::from_millis(1200)).contains(&duration),
        "expected a roughly 1-second final timeline, got {duration:?}"
    );
}

/// A capture holding real frames whose container declares a near-zero timeline
/// plays back as an empty video. A stream copy cannot recover a timeline the
/// frames themselves don't carry, so the recording is rejected instead of
/// becoming an upload candidate off the capture's wall-clock duration.
#[tokio::test]
async fn rejects_a_capture_whose_container_reports_no_timeline() {
    if !ffmpeg_available().await {
        return;
    }

    let path = std::env::temp_dir().join(format!(
        "warp-zero-length-recording-test-{}.mp4",
        uuid::Uuid::new_v4()
    ));
    // Twenty real frames spanning a ~0.02s container timeline, matching the
    // frames-present / duration-near-zero capture this guard exists for.
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=16x16:rate=1000:duration=1",
            "-frames:v",
            "20",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .output()
        .await
        .expect("run ffmpeg");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result = playable_video(&path).await;
    let remux_path = path.with_extension("remux.mp4");
    let remux_left_behind = remux_path.exists();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&remux_path);

    assert!(
        matches!(result, Err(RecordingError::Finalize { .. })),
        "a zero-length timeline should not be publishable, got {result:?}"
    );
    assert!(
        !remux_left_behind,
        "the failed repair file should be removed"
    );
}
