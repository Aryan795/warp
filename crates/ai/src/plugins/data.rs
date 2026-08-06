//! Persistent `PLUGIN_DATA` directories for plugin instances.
//!
//! Agent Plugins §9.1 requires `PLUGIN_DATA` to be outside the package, writable, dedicated to
//! one installed plugin instance, and preserved when the package contents change. The directory
//! is therefore keyed by identity that survives an update — frontend, source, scope, and manifest
//! name — and deliberately excludes the manifest version and any digest of package contents.
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::identity::PluginInstanceId;

/// Absolute directory a Factory worker provides for durable plugin data.
///
/// A worker that cannot provide a writable persistent root must fail dispatch rather than fall
/// back to ephemeral storage, which would break the persistence guarantee in §9.1.
pub const PLUGIN_DATA_ROOT_ENV: &str = "WARP_PLUGIN_DATA_ROOT";

/// Which front-end owns a plugin runtime instance.
///
/// The GUI and the TUI discover the same packages but do not share running MCP processes or
/// writable plugin state, matching the existing frontend-specific MCP state boundary. Two
/// concurrently running client versions must not mutate one plugin's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginFrontend {
    Gui,
    Tui,
}

impl PluginFrontend {
    fn key_token(self) -> &'static str {
        match self {
            PluginFrontend::Gui => "gui",
            PluginFrontend::Tui => "tui",
        }
    }
}

/// Resolves the persistent data directory for a plugin instance.
pub trait PluginDataLocator {
    /// Returns the instance's data directory without creating it.
    fn data_dir(&self, instance: &PluginInstanceId) -> PathBuf;

    /// Creates the instance's data directory and returns it.
    ///
    /// Called immediately before the first stdio start for the instance, never during discovery
    /// or validation.
    fn ensure_data_dir(&self, instance: &PluginInstanceId) -> io::Result<PathBuf> {
        let dir = self.data_dir(instance);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// The filesystem-safe key that identifies one plugin instance's data directory.
///
/// Stable across package updates and distinct across frontends, sources, scopes, and names.
pub fn plugin_data_instance_key(frontend: PluginFrontend, instance: &PluginInstanceId) -> String {
    let mut hasher = Sha256::new();
    // Length-prefix each field so that two different splits of the same concatenated bytes
    // cannot collide (e.g. scope "agent/a" + name "b" versus scope "agent" + name "a/b").
    for field in [
        frontend.key_token(),
        &format!("{:?}", instance.source.kind),
        &instance.source.stable_identity,
        &instance.scope.key_token(),
        &instance.manifest_name,
    ] {
        hasher.update(field.len().to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        })
}

/// Locates plugin data under a base directory owned by the active frontend.
#[derive(Debug, Clone)]
pub struct LocalPluginDataLocator {
    base: PathBuf,
    frontend: PluginFrontend,
}

impl LocalPluginDataLocator {
    /// Creates a locator rooted at `<base>/plugins/data`.
    ///
    /// Interactive clients pass `warp_core::paths::data_dir()`; a Factory worker passes the
    /// durable root it advertised through [`PLUGIN_DATA_ROOT_ENV`].
    pub fn new(base: impl AsRef<Path>, frontend: PluginFrontend) -> Self {
        Self {
            base: base.as_ref().to_path_buf(),
            frontend,
        }
    }

    /// The directory that holds every instance's data for this locator.
    pub fn root(&self) -> PathBuf {
        self.base.join("plugins").join("data")
    }
}

impl PluginDataLocator for LocalPluginDataLocator {
    fn data_dir(&self, instance: &PluginInstanceId) -> PathBuf {
        self.root()
            .join(plugin_data_instance_key(self.frontend, instance))
    }
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
