use super::JsonlDecoder;

fn decode(chunks: &[&[u8]]) -> Vec<u8> {
    let mut decoder = JsonlDecoder::new();
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(decoder.push(chunk));
    }
    out.extend(decoder.finish());
    out
}

fn event(event_type: &str, text: &str) -> String {
    serde_json::json!({
        "type": event_type,
        "level": 3,
        "timestamp": 1,
        "text": text,
    })
    .to_string()
}

#[test]
fn raw_events_preserve_cr_overwrite_bytes() {
    let line1 = event("raw", "#15 extracting sha256:abc 1.5MB / 52.40MB");
    let line2 = event("raw", "\r#15 extracting sha256:abc 52.40MB / 52.40MB");
    let input = format!("{line1}\n{line2}\n");
    let out = decode(&[input.as_bytes()]);
    assert_eq!(
        out,
        b"#15 extracting sha256:abc 1.5MB / 52.40MB\r#15 extracting sha256:abc 52.40MB / 52.40MB"
    );
}

#[test]
fn raw_events_preserve_cursor_up_sequences() {
    let first = event("raw", "layer-a\nlayer-b");
    let update = event("raw", "\u{1b}[1A\rlayer-A");
    let input = format!("{first}\n{update}\n");
    let out = decode(&[input.as_bytes()]);
    assert_eq!(out, b"layer-a\nlayer-b\x1b[1A\rlayer-A");
}

#[test]
fn text_events_become_crlf_terminated_lines() {
    let input = format!("{}\n", event("text", "step-one"));
    assert_eq!(decode(&[input.as_bytes()]), b"step-one\r\n");
}

#[test]
fn start_events_render_as_text_and_stop_progress_are_dropped() {
    let start = event("start", "Run: docker build");
    let stop = serde_json::json!({
        "type": "stop",
        "level": 3,
        "timestamp": 2,
        "text": "Run: docker build",
        "startTimestamp": 1,
    })
    .to_string();
    let progress = serde_json::json!({
        "type": "progress",
        "name": "Building image",
        "status": "running",
    })
    .to_string();
    let input = format!("{start}\n{stop}\n{progress}\n");
    assert_eq!(decode(&[input.as_bytes()]), b"Run: docker build\r\n");
}

#[test]
fn splits_json_lines_across_chunks() {
    let line = event("raw", "\rnext");
    let bytes = format!("{line}\n");
    let split_at = bytes.len() / 2;
    let out = decode(&[&bytes.as_bytes()[..split_at], &bytes.as_bytes()[split_at..]]);
    assert_eq!(out, b"\rnext");
}

#[test]
fn leftover_non_json_is_left_aligned() {
    let out = decode(&[b"Cannot connect to the Docker daemon\n"]);
    assert_eq!(out, b"Cannot connect to the Docker daemon\r\n");
}
