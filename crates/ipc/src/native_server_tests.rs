use super::server::windows_named_pipe_path;

// This transformation must exactly match what `interprocess`'s `LocalSocketListener` does
// internally for a plain (non-namespaced) name, or `restrict_named_pipe_to_current_user`
// (REV-1546) would silently target the wrong (or a nonexistent) pipe object.
#[test]
fn builds_windows_named_pipe_path_from_plain_name() {
    assert_eq!(
        windows_named_pipe_path("WarpDefault_URI_CHANNEL"),
        r"\\.\pipe\WarpDefault_URI_CHANNEL"
    );
}
