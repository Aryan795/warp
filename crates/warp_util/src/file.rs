use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_lite::io::AsyncReadExt as _;
use warp_errors::{ErrorExt, register_error};

#[derive(thiserror::Error, Debug)]
pub enum FileSaveError {
    #[error("No file path associated with file when saving file {0:?}")]
    NoFilePath(FileId),
    #[error("IO error when saving file.")]
    IOError {
        #[source]
        error: io::Error,
        path: PathBuf,
    },
    #[error("Remote file operation failed: {0}")]
    RemoteError(String),
    /// A non-IO failure with a self-describing message (e.g. content could
    /// not be derived for the write).
    #[error("{0}")]
    Other(String),
}

impl ErrorExt for FileSaveError {
    fn is_actionable(&self) -> bool {
        match self {
            FileSaveError::NoFilePath(_) | FileSaveError::Other(_) => true,
            FileSaveError::IOError { .. } | FileSaveError::RemoteError(_) => false,
        }
    }
}
register_error!(FileSaveError);

/// Maximum size, in bytes, of a file that can be fully loaded into memory as a
/// `String` (e.g. to populate an editor buffer). Reading larger files whole
/// risks multi-gigabyte allocations for pathologically large files (logs,
/// binaries opened by mistake, etc.); callers should check
/// [`FileLoadError::TooLarge`] and surface a friendly error instead of
/// attempting the read.
pub const MAX_LOADABLE_FILE_SIZE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(thiserror::Error, Debug)]
pub enum FileLoadError {
    #[error("File does not exist")]
    DoesNotExist,
    #[error("IO error when loading file.")]
    IOError(#[from] io::Error),
    #[error("File is too large to open ({size_bytes} bytes, limit is {limit_bytes} bytes)")]
    TooLarge { size_bytes: u64, limit_bytes: u64 },
}

/// Reads `path` into a `Vec<u8>`, rejecting content exceeding `max_bytes`.
///
/// The cap is enforced by the read itself — at most `max_bytes + 1` bytes are read from a
/// single open handle via [`AsyncReadExt::take`] — rather than by a preceding `stat`. A
/// `stat`-then-reread-by-path check cannot be trusted as the enforcement boundary: a FIFO,
/// `/dev/zero`, or other virtual stream commonly reports a length of zero regardless of how
/// much data it actually yields, and a regular file's size can change (or the path can be
/// atomically replaced) between the `stat` and a later, separate open. `metadata` is still
/// queried here, but only as a best-effort allocation-size hint, capped so an inflated or
/// unrelated `stat` length can never itself cause an over-cap reservation.
pub async fn read_capped(path: &Path, max_bytes: u64) -> Result<Vec<u8>, FileLoadError> {
    let file = async_fs::File::open(path).await?;
    let read_limit = max_bytes.saturating_add(1);
    let metadata_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let mut buffer = Vec::with_capacity(metadata_len.min(read_limit) as usize);
    file.take(read_limit).read_to_end(&mut buffer).await?;
    if buffer.len() as u64 > max_bytes {
        // `stat` isn't trusted for the accept/reject decision above, but when it reports a size
        // at least as large as what was actually read, it's more informative to surface than the
        // bare "read past the limit" lower bound.
        let size_bytes = metadata_len.max(buffer.len() as u64);
        return Err(FileLoadError::TooLarge {
            size_bytes,
            limit_bytes: max_bytes,
        });
    }
    Ok(buffer)
}

/// String counterpart of [`read_capped`]; see its doc comment for the enforcement rationale.
pub async fn read_to_string_capped(path: &Path, max_bytes: u64) -> Result<String, FileLoadError> {
    let bytes = read_capped(path, max_bytes).await?;
    String::from_utf8(bytes)
        .map_err(|err| FileLoadError::IOError(io::Error::new(io::ErrorKind::InvalidData, err)))
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileId(usize);

impl FileId {
    /// Constructs a new globally-unique file ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> FileId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        FileId(raw)
    }
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
