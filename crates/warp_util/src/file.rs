use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use warp_errors::{ErrorExt, register_error};

/// Reads `path` into a `Vec<u8>`, rejecting files above `max_bytes` before reading.
///
/// `async_fs::read` reserves the file's entire on-disk size up front via
/// `Vec::with_capacity`, with no upper bound. A pathologically large or sparse file can
/// therefore balloon the process's memory footprint by tens of GiB in a single allocation
/// before a single byte is read. Checking `metadata` first avoids that reservation. See
/// APP-4801.
pub async fn read_capped(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let len = async_fs::metadata(path).await?.len();
    if len > max_bytes {
        return Err(io::Error::other(format!(
            "file is too large to load into memory: {len} bytes (limit {max_bytes} bytes)"
        )));
    }
    async_fs::read(path).await
}

/// String counterpart of [`read_capped`]; see its doc comment for the rationale.
pub async fn read_to_string_capped(path: &Path, max_bytes: u64) -> io::Result<String> {
    let len = async_fs::metadata(path).await?.len();
    if len > max_bytes {
        return Err(io::Error::other(format!(
            "file is too large to load into memory: {len} bytes (limit {max_bytes} bytes)"
        )));
    }
    async_fs::read_to_string(path).await
}

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

#[derive(thiserror::Error, Debug)]
pub enum FileLoadError {
    #[error("File does not exist")]
    DoesNotExist,
    #[error("IO error when loading file.")]
    IOError(#[from] io::Error),
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
