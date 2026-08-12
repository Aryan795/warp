use super::*;

#[test]
fn parses_showinfo_pts_times_from_stderr() {
    let stderr = "\
frame=    1 fps=0.0 q=-0.0 size=N/A time=00:00:00.00 bitrate=N/A speed=N/A
[Parsed_showinfo_1 @ 0x600001234] n:0 pts:0 pts_time:0 duration:1001
[Parsed_showinfo_1 @ 0x600001234] n:1 pts:1502 pts_time:3.128 duration:1001
[Parsed_showinfo_1 @ 0x600001234] n:2 pts:3600 pts_time:7.5 duration:1001
";
    assert_eq!(
        parse_showinfo_pts_times(stderr),
        vec![0.0, 3.128, 7.5],
        "should extract pts_time from every showinfo line, ignoring unrelated stderr noise"
    );
}

#[test]
fn parses_showinfo_pts_times_returns_empty_when_no_scene_changes_detected() {
    let stderr = "frame=    0 fps=0.0 q=-0.0 Lsize=N/A time=00:00:00.00 bitrate=N/A speed=N/A";
    assert!(parse_showinfo_pts_times(stderr).is_empty());
}

#[test]
fn parses_ffmpeg_duration_from_stderr_banner() {
    let stderr = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'test.mp4':\n  Duration: 00:01:06.04, start: 0.000000, bitrate: 91 kb/s\n";
    assert_eq!(parse_ffmpeg_duration_secs(stderr), Some(66.04));
}

#[test]
fn parses_ffmpeg_duration_returns_none_without_a_duration_line() {
    let stderr = "ffmpeg version 6.0\nsome unrelated stderr output\n";
    assert_eq!(parse_ffmpeg_duration_secs(stderr), None);
}

#[test]
fn evenly_spaced_timestamps_excludes_endpoints() {
    let timestamps = evenly_spaced_timestamps(10.0, 3);
    assert_eq!(timestamps, vec![2.5, 5.0, 7.5]);
    for t in &timestamps {
        assert!(*t > 0.0 && *t < 10.0);
    }
}

#[test]
fn evenly_spaced_timestamps_with_zero_count_is_empty() {
    assert!(evenly_spaced_timestamps(10.0, 0).is_empty());
}

#[test]
fn supported_video_mime_types_accepts_common_containers_only() {
    assert!(is_supported_video_mime_type("video/mp4"));
    assert!(is_supported_video_mime_type("video/quicktime"));
    assert!(is_supported_video_mime_type("video/webm"));
    assert!(!is_supported_video_mime_type("image/png"));
    assert!(!is_supported_video_mime_type("video/mpeg"));
}

#[test]
fn frame_and_density_floor_constants_stay_under_the_query_image_limit() {
    // Video-as-context reuses the image-as-context pipeline, which caps a single query at 20
    // images. Both constants must stay comfortably below that shared limit.
    assert!(MAX_VIDEO_FRAMES <= 16);
    assert!(MIN_VIDEO_FRAMES < MAX_VIDEO_FRAMES);
}

#[test]
fn is_supported_video_filepath_checks_extension() {
    assert!(is_supported_video_filepath("/tmp/clip.mp4"));
    assert!(is_supported_video_filepath("/tmp/clip.mov"));
    assert!(!is_supported_video_filepath("/tmp/photo.png"));
    assert!(!is_supported_video_filepath("/tmp/clip.mpeg"));
}

#[test]
fn capped_frame_count_is_unrestricted_with_no_existing_attachments() {
    assert_eq!(capped_frame_count(16, 0, 0, 20, 200), 16);
}

#[test]
fn capped_frame_count_reserves_room_for_images_already_pending_on_the_query() {
    // Regression test: with 10 images already pending on the query, attaching 16 more frames
    // unconditionally would push the query to 26 images, well past the server's 20-per-query
    // limit (which then rejects the whole request). The cap must leave only the 10 remaining
    // slots for frames.
    assert_eq!(capped_frame_count(16, 10, 0, 20, 200), 10);
}

#[test]
fn capped_frame_count_reserves_room_for_the_conversation_limit() {
    // Regression test: near the 200-image conversation limit, the conversation limit is more
    // restrictive than the per-query limit and must win.
    assert_eq!(capped_frame_count(16, 0, 195, 20, 200), 5);
}

#[test]
fn capped_frame_count_is_zero_when_the_query_is_already_at_the_limit() {
    assert_eq!(capped_frame_count(16, 20, 0, 20, 200), 0);
    assert_eq!(capped_frame_count(16, 0, 200, 20, 200), 0);
}

#[test]
fn transcript_still_applies_when_all_frames_are_still_pending() {
    let expected = vec![
        "video.mp4-frame-01.jpg".to_string(),
        "video.mp4-frame-02.jpg".to_string(),
    ];
    let pending = vec![
        "video.mp4-frame-01.jpg".to_string(),
        "video.mp4-frame-02.jpg".to_string(),
        "unrelated.png".to_string(),
    ];
    assert!(transcript_still_applies(&expected, &pending));
}

#[test]
fn transcript_does_not_apply_after_the_frames_were_already_sent() {
    // Regression test for the send-before-transcript race: once a query is sent, its pending
    // image attachments (including the video's frames) are drained from the context model. A
    // transcript that resolves afterwards must detect that its frames are gone and refuse to
    // land in whatever the composer now contains (e.g. the user's next prompt).
    let expected = vec![
        "video.mp4-frame-01.jpg".to_string(),
        "video.mp4-frame-02.jpg".to_string(),
    ];
    let pending_after_send: Vec<String> = vec![];
    assert!(!transcript_still_applies(&expected, &pending_after_send));

    // Also refuse if only some of the frames are still pending (e.g. the user removed one).
    let partially_pending = vec!["video.mp4-frame-01.jpg".to_string()];
    assert!(!transcript_still_applies(&expected, &partially_pending));
}

#[test]
fn transcript_does_not_apply_with_no_expected_frames() {
    // Defensive: an empty expectation should never be treated as "still applies".
    assert!(!transcript_still_applies(
        &[],
        &["anything.jpg".to_string()]
    ));
}
