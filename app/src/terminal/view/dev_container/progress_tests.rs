use super::ProgressCollapser;

fn collapse(chunks: &[&[u8]]) -> Vec<u8> {
    let mut collapser = ProgressCollapser::new();
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(collapser.push(chunk));
    }
    out.extend(collapser.finish());
    out
}

fn overwrite_count(out: &[u8]) -> usize {
    out.windows(b"\r\x1b[K".len())
        .filter(|w| *w == b"\r\x1b[K")
        .count()
}

#[test]
fn collapses_buildkit_extracting_snapshots_for_the_same_vertex() {
    let input = b"[2026-09-02T00:43:15.960Z] #15 extracting sha256:abc 1.5MB / 52.40MB\n\
[2026-09-02T00:43:16.100Z] #15 extracting sha256:abc 4.6MB / 52.40MB\n\
[2026-09-02T00:43:16.400Z] #15 extracting sha256:abc 52.40MB / 52.40MB\n\
[2026-09-02T00:43:16.500Z] #15 DONE 2.1s\n";
    let out = collapse(&[input]);
    assert!(
        overwrite_count(&out) >= 2,
        "matching snapshots must overwrite with CR, got {out:?}"
    );
    assert!(
        out.windows(b"52.40MB / 52.40MB".len())
            .any(|w| w == b"52.40MB / 52.40MB")
    );
    assert!(
        out.windows(b"#15 DONE 2.1s".len())
            .any(|w| w == b"#15 DONE 2.1s")
    );
}

#[test]
fn keeps_distinct_vertices_and_ordinary_logs() {
    let input = b"step-one\n\
#14 extracting sha256:aaa 1MB / 2MB\n\
#15 extracting sha256:bbb 1MB / 8MB\n\
#15 extracting sha256:bbb 8MB / 8MB\n\
step-two\n";
    let out = collapse(&[input]);
    assert!(out.windows(b"step-one".len()).any(|w| w == b"step-one"));
    assert!(out.windows(b"step-two".len()).any(|w| w == b"step-two"));
    assert!(
        out.windows(b"#14 extracting sha256:aaa".len())
            .any(|w| w == b"#14 extracting sha256:aaa")
    );
    assert_eq!(
        overwrite_count(&out),
        1,
        "only the repeated vertex should overwrite, got {out:?}"
    );
}

#[test]
fn collapses_across_chunk_boundaries() {
    let chunks: &[&[u8]] = &[
        b"[2026-09-02T00:00:00.000Z] #15 extracting sha256:abc 1.5MB / 52.40MB\n[2026-09-02T00:00:00.100Z] #15 extract",
        b"ing sha256:abc 4.6MB / 52.40MB\n",
    ];
    let out = collapse(chunks);
    assert!(
        overwrite_count(&out) >= 1,
        "split snapshots must still overwrite, got {out:?}"
    );
    assert!(
        out.windows(b"4.6MB / 52.40MB".len())
            .any(|w| w == b"4.6MB / 52.40MB")
    );
}

#[test]
fn streams_cap_exceeding_progress_looking_line_without_lf() {
    let mut line = b"#15 extracting sha256:abc ".to_vec();
    line.resize(600, b'x');
    let mut collapser = ProgressCollapser::new();
    let pushed = collapser.push(&line);
    assert_eq!(
        pushed,
        line,
        "a cap-exceeding no-LF line must stream through, got {} bytes",
        pushed.len()
    );
    assert!(
        collapser.finish().is_empty(),
        "finish must not invent a trailing snapshot after pass-through"
    );
}

#[test]
fn collapses_cr_only_classic_layer_snapshots() {
    let out = collapse(&[b"17a39c0ba978: Downloading 1MB\r17a39c0ba978: Downloading 2MB\r"]);
    assert_eq!(
        overwrite_count(&out),
        1,
        "CR-only snapshots for the same layer must overwrite, got {out:?}"
    );
    assert!(
        out.windows(b"Downloading 2MB".len())
            .any(|w| w == b"Downloading 2MB")
    );
}

#[test]
fn recovers_same_vertex_collapse_after_pass_through() {
    let mut collapser = ProgressCollapser::new();
    let noise = vec![b'g'; 600];
    let streamed = collapser.push(&noise);
    assert_eq!(streamed, noise, "non-progress bytes must stream before LF");

    let rest = b"\n#15 extracting sha256:abc 1MB / 2MB\n#15 extracting sha256:abc 2MB / 2MB\n";
    let mut out = streamed;
    out.extend(collapser.push(rest));
    out.extend(collapser.finish());
    assert_eq!(
        overwrite_count(&out),
        1,
        "collapse must resume after pass-through, got {out:?}"
    );
    assert!(out.windows(b"2MB / 2MB".len()).any(|w| w == b"2MB / 2MB"));
}
