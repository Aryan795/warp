use std::io::ErrorKind;

use futures_lite::future::block_on;
use tempfile::TempDir;

use super::{read_capped, read_to_string_capped};

fn write_file(dir: &TempDir, name: &str, contents: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).expect("write temp file");
    path
}

#[test]
fn read_to_string_capped_reads_file_under_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "small.txt", b"hello world");

    let contents = block_on(read_to_string_capped(&path, 1024)).expect("should read file");
    assert_eq!(contents, "hello world");
}

#[test]
fn read_to_string_capped_rejects_file_over_limit() {
    // Regression for APP-4801: reading a file whose on-disk size exceeds the cap must not
    // attempt to reserve a String of that size; it should be rejected up front.
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "big.txt", &vec![b'a'; 2048]);

    let error = block_on(read_to_string_capped(&path, 1024)).expect_err("should reject");
    assert_ne!(error.kind(), ErrorKind::NotFound);
    assert!(error.to_string().contains("too large"), "got: {error}");
}

#[test]
fn read_to_string_capped_missing_file_reports_not_found() {
    let dir = TempDir::new().expect("create tempdir");
    let missing = dir.path().join("does-not-exist.txt");

    let error = block_on(read_to_string_capped(&missing, 1024)).expect_err("should fail");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn read_capped_reads_file_under_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "small.bin", &[1, 2, 3, 4]);

    let contents = block_on(read_capped(&path, 1024)).expect("should read file");
    assert_eq!(contents, vec![1, 2, 3, 4]);
}

#[test]
fn read_capped_rejects_file_over_limit() {
    let dir = TempDir::new().expect("create tempdir");
    let path = write_file(&dir, "big.bin", &vec![0u8; 2048]);

    let error = block_on(read_capped(&path, 1024)).expect_err("should reject");
    assert_ne!(error.kind(), ErrorKind::NotFound);
    assert!(error.to_string().contains("too large"), "got: {error}");
}
