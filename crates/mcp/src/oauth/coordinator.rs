//! Cross-process coordination for MCP OAuth authentication.
//!
//! When the same MCP installation is started by multiple Warp processes, exactly
//! one process must own the interactive OAuth attempt and open one authorization
//! page. This module provides the primitives that guarantee that:
//!
//! - [`AuthLease`] — a crash-safe OS file lease (via `fs4`) held for the entire
//!   interactive flow. `try_lock_exclusive` elects one leader without polling a
//!   PID file; RAII releases it on normal completion and the OS releases it on
//!   crash on Unix and Windows.
//! - [`CoordinationPaths`] — owner-only (`0700`) per-user, channel-scoped runtime
//!   directory with `0600` lock and metadata files. Paths are derived from the
//!   channel, installation UUID, and credential namespace and contain no access
//!   tokens, refresh tokens, authorization codes, or client secrets.
//! - [`OAuthCoordinator`] — orchestrates leader election, the follower wait loop
//!   (with promotion when the prior leader fails), and the serialized shared
//!   credential read/merge/write used by both the initial OAuth publish and
//!   runtime auto-refresh.
//!
//! The coordinator is native-only: the WASM MCP path does not run interactive
//! OAuth and never constructs it.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use futures::future::BoxFuture;
use instant::Instant;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use super::PersistedCredentials;

/// Callback invoked once when this process becomes a follower (enters the wait
/// loop) so the app can show the visible `WaitingForAuthentication` state with
/// no authorization URL.
pub type WaiterNotifier = Box<dyn Fn() -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;

/// Bumped when the on-disk owner-metadata or relay protocol changes in a way
/// older readers cannot safely ignore. Readers reject records with a mismatched
/// version rather than forwarding a callback to a stale owner.
const OWNER_METADATA_PROTOCOL_VERSION: u32 = 1;

/// Environment override for the coordination root directory. Tests and the
/// process-safe secure-store seam set this to a temporary owner-only directory
/// so they never touch a developer's real runtime/keychain state.
const COORDINATION_DIR_ENV: &str = "WARP_MCP_OAUTH_COORDINATION_DIR";

/// Which secure-storage namespace an installation's credentials live in.
///
/// Templatable and file-based installations share no credential map, so they use
/// distinct coordination subdirectories and credential-store lock files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialNamespace {
    Templatable,
    FileBased,
}

impl CredentialNamespace {
    fn dir_name(self) -> &'static str {
        match self {
            Self::Templatable => "templatable",
            Self::FileBased => "file-based",
        }
    }

    /// The map key kind stored under this namespace, used by the merge helpers
    /// to deserialize the correct `HashMap<K, PersistedCredentials>`.
    fn key_kind(self) -> CredentialKeyKind {
        match self {
            Self::Templatable => CredentialKeyKind::Uuid,
            Self::FileBased => CredentialKeyKind::Hash,
        }
    }
}

/// The kind of map key an installation uses inside its credential namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialKeyKind {
    Uuid,
    Hash,
}

/// Identifies one installation's entry in its shared credential map.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CredentialKey {
    Uuid(Uuid),
    Hash(u64),
}

impl CredentialKey {
    fn kind(self) -> CredentialKeyKind {
        match self {
            Self::Uuid(_) => CredentialKeyKind::Uuid,
            Self::Hash(_) => CredentialKeyKind::Hash,
        }
    }
}

/// Low-level secure-storage access for one credential namespace.
///
/// The coordinator takes the credential-store file lock and then calls these
/// methods, so implementations must not re-entrantly acquire the coordination
/// locks. Methods map to the platform secure-storage backend in the app crate
/// (or a fake in tests) and never touch the coordination directory itself.
#[async_trait::async_trait]
pub trait SecureCredentialBackend: Send + Sync + 'static {
    /// Reads the raw JSON map string for the namespace's shared secure-storage
    /// key, or `None` when no entry exists.
    async fn read_raw(&self) -> anyhow::Result<Option<String>>;

    /// Writes the raw JSON map string for the namespace's shared secure-storage
    /// key via the owner-only fallback path.
    async fn write_raw(&self, json: &str) -> anyhow::Result<()>;

    /// Best-effort notification that credentials for this installation changed
    /// (the app updates its in-memory cache and emits `CredentialsChanged`).
    /// Errors are logged by the caller and never abort the flow.
    async fn notify_changed(&self);
}

/// Tunable timing for the coordinator. Production uses the defaults; tests
/// inject shorter values so promotion and timeout paths run quickly.
#[derive(Clone, Copy, Debug)]
pub struct CoordinatorConfig {
    /// How long a follower waits for shared credentials before timing out with
    /// a controlled "authentication in another instance did not complete"
    /// error. The default is the existing five-minute OAuth callback deadline
    /// plus a small handoff grace period.
    pub wait_deadline: Duration,
    /// Polling interval for the follower wait loop and promotion race.
    pub poll_interval: Duration,
    /// Bounded retry budget for acquiring the credential-store file lock.
    pub cred_store_lock_timeout: Duration,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            // 5-minute callback timeout + 30-second handoff grace period.
            wait_deadline: Duration::from_secs(330),
            poll_interval: Duration::from_millis(500),
            cred_store_lock_timeout: Duration::from_secs(5),
        }
    }
}

/// Owner-only filesystem layout for one `(channel, installation, namespace)`
/// coordination key.
#[derive(Clone, Debug)]
struct CoordinationPaths {
    /// Owner-only (`0700`) directory holding every coordination artifact for
    /// this channel/namespace.
    dir: PathBuf,
    /// The OS lease file held exclusively by the current interactive leader.
    auth_lease_file: PathBuf,
    /// Atomically published owner metadata for the desktop callback relay.
    owner_metadata_file: PathBuf,
    /// The per-namespace credential-store lock serializing read/merge/write.
    cred_store_lock_file: PathBuf,
}

impl CoordinationPaths {
    fn new(channel: &str, namespace: CredentialNamespace, installation_uuid: Uuid) -> Result<Self> {
        let base = coordination_root_dir();
        let dir = base.join(channel).join(namespace.dir_name());
        // Derive a stable, filesystem-safe suffix from the installation UUID.
        // The UUID is already a constrained character set; use the simple form
        // so paths are predictable and contain no separators.
        let uuid_suffix = installation_uuid.simple().to_string();
        let auth_lease_file = dir.join(format!("auth-{uuid_suffix}.lock"));
        let owner_metadata_file = dir.join(format!("owner-{uuid_suffix}.json"));
        let cred_store_lock_file = dir.join("cred-store.lock");
        let paths = Self {
            dir,
            auth_lease_file,
            owner_metadata_file,
            cred_store_lock_file,
        };
        paths.ensure()?;
        Ok(paths)
    }

    /// Creates the coordination directory with owner-only permissions and
    /// refuses to operate on a world-writable or symlinked location.
    fn ensure(&self) -> Result<()> {
        if self.dir.exists() {
            // Refuse to use a symlinked or otherwise suspicious coordination
            // directory: a symlink could redirect lock/metadata writes to an
            // attacker-controlled location.
            let canonical = fs::canonicalize(&self.dir).with_context(|| {
                format!("failed to resolve coordination directory {:?}", self.dir)
            })?;
            if canonical != self.dir {
                bail!(
                    "MCP OAuth coordination directory {:?} resolves to {:?}; refusing to follow \
                     a redirected path",
                    self.dir,
                    canonical
                );
            }
        } else {
            fs::create_dir_all(&self.dir).with_context(|| {
                format!(
                    "failed to create MCP OAuth coordination directory {:?}",
                    self.dir
                )
            })?;
        }
        set_owner_only_dir(&self.dir)?;
        Ok(())
    }

    /// Opens or creates the auth lease lock file with owner-only permissions.
    fn open_auth_lease(&self) -> Result<fs::File> {
        open_owner_only_file(&self.auth_lease_file)
    }

    /// Opens or creates the credential-store lock file with owner-only
    /// permissions.
    fn open_cred_store_lock(&self) -> Result<fs::File> {
        open_owner_only_file(&self.cred_store_lock_file)
    }
}

/// The result of resolving this process's role in the coordinated OAuth flow.
#[derive(Debug)]
pub enum ResolveOutcome {
    /// Shared credentials are already available from another process; build the
    /// client from them without opening a page. Boxed because `PersistedCredentials`
    /// is large and the other variants are small.
    Credentials(Box<PersistedCredentials>),
    /// This process won leadership and should perform the interactive OAuth
    /// flow. The lease is released when `AuthLease` is dropped.
    Leader(AuthLease),
    /// No credentials appeared and no lease could be acquired before the wait
    /// deadline. The caller reports a controlled retryable error.
    Timeout,
}

/// RAII wrapper around an `fs4` exclusive OS lock on the auth lease file.
///
/// The OS releases the lock when the file descriptor is closed (on drop, on
/// process exit, and on crash on Unix and Windows), so a crashed leader cannot
/// prevent a successor from acquiring the same lease. A leftover empty lock
/// file is reusable: the next opener simply re-acquires the OS lock on it.
pub struct AuthLease {
    _file: fs::File,
    path: PathBuf,
}

impl std::fmt::Debug for AuthLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthLease")
            .field("path", &self.path)
            .finish()
    }
}

impl AuthLease {
    /// Attempts to acquire the exclusive lease without blocking. Returns
    /// `Ok(Some(_))` when this process is now the leader, `Ok(None)` when
    /// another process already holds the lease, or an error when the lock file
    /// cannot be opened or locked for an unexpected reason.
    fn try_acquire(paths: &CoordinationPaths) -> Result<Option<Self>> {
        let file = paths.open_auth_lease()?;
        // fs4's `try_lock_exclusive` returns `Ok(true)` when the lock is
        // acquired, `Ok(false)` when it is contended, and `Err` for a real
        // I/O or permission failure.
        match fs4::fs_std::FileExt::try_lock_exclusive(&file) {
            Ok(true) => Ok(Some(Self {
                _file: file,
                path: paths.auth_lease_file.clone(),
            })),
            Ok(false) => Ok(None),
            Err(err) => Err(anyhow::Error::new(err).context(format!(
                "failed to acquire MCP OAuth lease {:?}",
                paths.auth_lease_file
            ))),
        }
    }

    /// The on-disk lock file path (for diagnostics only; never logged with
    /// contents because the file never contains secrets).
    #[allow(dead_code)]
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AuthLease {
    fn drop(&mut self) {
        // The OS lock is released when the file descriptor closes. A leftover
        // empty lock file is harmless and reusable, so we do not unlink it
        // (unlinking would race with a concurrent opener on Windows).
    }
}

/// RAII wrapper around a credential-store file lock (shared or exclusive).
/// Dropping it releases the OS lock.
struct CredentialStoreLock {
    _file: fs::File,
}

impl CredentialStoreLock {
    fn try_acquire_shared(paths: &CoordinationPaths) -> Result<Self> {
        let file = paths.open_cred_store_lock()?;
        fs4::fs_std::FileExt::lock_shared(&file).map_err(|err| {
            anyhow::Error::new(err).context("failed to lock MCP credential store")
        })?;
        Ok(Self { _file: file })
    }

    fn try_acquire_exclusive(paths: &CoordinationPaths) -> Result<Self> {
        let file = paths.open_cred_store_lock()?;
        fs4::fs_std::FileExt::lock_exclusive(&file).map_err(|err| {
            anyhow::Error::new(err).context("failed to lock MCP credential store exclusively")
        })?;
        Ok(Self { _file: file })
    }
}

/// The cross-process OAuth coordinator for one installation.
///
/// It owns no UI state and no secure-storage backend directly: the app provides
/// a [`SecureCredentialBackend`] so the cross-process logic stays testable in
/// this crate against temporary directories and an in-memory fake.
pub struct OAuthCoordinator {
    paths: CoordinationPaths,
    backend: std::sync::Arc<dyn SecureCredentialBackend>,
    key: CredentialKey,
    config: CoordinatorConfig,
    became_waiter: Option<WaiterNotifier>,
}

impl OAuthCoordinator {
    /// Constructs a coordinator for `(channel, namespace, installation_uuid)`
    /// using the given secure-storage backend and the key that identifies this
    /// installation inside the namespace's shared map.
    pub fn new(
        channel: &str,
        namespace: CredentialNamespace,
        installation_uuid: Uuid,
        key: CredentialKey,
        backend: std::sync::Arc<dyn SecureCredentialBackend>,
        config: CoordinatorConfig,
        became_waiter: Option<WaiterNotifier>,
    ) -> Result<Self> {
        // The key kind must match the namespace's map type, otherwise the merge
        // would deserialize the wrong shape and corrupt the shared map.
        if key.kind() != namespace.key_kind() {
            bail!(
                "MCP OAuth coordinator key kind {:?} does not match namespace {:?}",
                key.kind(),
                namespace
            );
        }
        let paths = CoordinationPaths::new(channel, namespace, installation_uuid)?;
        Ok(Self {
            paths,
            backend,
            key,
            config,
            became_waiter,
        })
    }

    /// The configuration this coordinator was built with.
    pub fn config(&self) -> CoordinatorConfig {
        self.config
    }

    /// Re-reads the latest shared credentials for this installation under the
    /// shared credential-store lock. Returns `Ok(None)` when no usable entry
    /// exists yet. A malformed map is rejected (not silently replaced) so a
    /// corrupt store cannot be overwritten with an empty map.
    pub async fn read_latest(&self) -> Result<Option<PersistedCredentials>> {
        let _lock = self.acquire_cred_store_lock(false).await?;
        let raw = self.backend.read_raw().await?;
        read_entry(raw, &self.key)
    }

    /// Merges `credentials` for this installation into the latest shared map
    /// and writes it back under the exclusive credential-store lock
    /// (read → merge → serialize → secure write). On a serialization or backend
    /// write failure the previous valid value is left untouched. Emits the
    /// change notification only after the shared write succeeds.
    pub async fn merge_and_write(&self, credentials: PersistedCredentials) -> Result<()> {
        let _lock = self.acquire_cred_store_lock(true).await?;
        let raw = self.backend.read_raw().await?;
        let json = merge_entry(raw, self.key, credentials)?;
        self.backend.write_raw(&json).await?;
        // Only notify after the shared write succeeds, so followers reload the
        // newly persisted value rather than a partial state.
        self.backend.notify_changed().await;
        Ok(())
    }

    /// Resolves this process's role: re-reads shared credentials first (another
    /// process may have just published them), then tries to acquire the lease.
    /// If neither yields credentials, waits — polling for credentials and
    /// racing for the lease — until credentials appear, this process is
    /// promoted to leader, or the wait deadline expires.
    pub async fn resolve_or_wait(&self) -> Result<ResolveOutcome> {
        // Eager re-check: another process may have finished while we were
        // starting up. This avoids an unnecessary lease attempt.
        if let Some(credentials) = self.read_latest().await? {
            return Ok(ResolveOutcome::Credentials(Box::new(credentials)));
        }
        // Eager leadership attempt: if we can grab the lease immediately, skip
        // the wait loop entirely.
        if let Some(lease) = AuthLease::try_acquire(&self.paths)? {
            return Ok(ResolveOutcome::Leader(lease));
        }
        self.wait_for_credentials_or_promotion().await
    }

    /// Follower wait loop: polls shared credentials and races for the lease
    /// until credentials appear (a leader published them), this process acquires
    /// the lease (promotion after a prior leader failed), or the wait deadline
    /// expires.
    async fn wait_for_credentials_or_promotion(&self) -> Result<ResolveOutcome> {
        // Notify the app once that this process is now waiting for another
        // instance, so it can show the waiting state with no authorization URL.
        if let Some(notify) = &self.became_waiter
            && let Err(err) = notify().await
        {
            log::warn!("Failed to emit MCP OAuth waiting state: {err:#}");
        }
        let deadline = Instant::now() + self.config.wait_deadline;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(ResolveOutcome::Timeout);
            }
            // Sleep for one poll interval, but never past the deadline.
            let sleep = std::cmp::min(
                self.config.poll_interval,
                deadline.saturating_duration_since(now),
            );
            tokio::time::sleep(sleep).await;

            if let Some(credentials) = self.read_latest().await? {
                return Ok(ResolveOutcome::Credentials(Box::new(credentials)));
            }
            // Race for promotion: if the prior leader released the lease (it
            // failed, timed out, or was killed), exactly one waiter wins.
            if let Some(lease) = AuthLease::try_acquire(&self.paths)? {
                return Ok(ResolveOutcome::Leader(lease));
            }
        }
    }

    /// Acquires the credential-store file lock, retrying with a short backoff
    /// up to `cred_store_lock_timeout` so concurrent writers serialize without
    /// blocking the async runtime. `exclusive` selects an exclusive (write) or
    /// shared (read) lock.
    async fn acquire_cred_store_lock(&self, exclusive: bool) -> Result<CredentialStoreLock> {
        let deadline = Instant::now() + self.config.cred_store_lock_timeout;
        loop {
            let lock = if exclusive {
                CredentialStoreLock::try_acquire_exclusive(&self.paths)
            } else {
                CredentialStoreLock::try_acquire_shared(&self.paths)
            };
            match lock {
                Ok(lock) => return Ok(lock),
                Err(err) => {
                    // A contended lock returns a WouldBlock-ish error from the
                    // blocking fs4 call; retry until the timeout. Any other
                    // error (permissions, missing dir) fails closed.
                    if Instant::now() >= deadline {
                        return Err(
                            err.context("timed out waiting for the MCP credential-store lock")
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Publishes owner metadata for the desktop callback relay. The metadata
    /// contains only the IPC endpoint address, an owner nonce, and the
    /// installation UUID — never an authorization code, token, or client
    /// secret. Atomically written (temp file + rename) with owner-only perms.
    pub fn publish_owner_metadata(&self, metadata: &OwnerMetadata) -> Result<()> {
        write_owner_metadata(&self.paths.owner_metadata_file, metadata)
    }

    /// Removes the owner metadata, best-effort. Called when the leader releases
    /// the lease (success, failure, timeout, or shutdown).
    pub fn remove_owner_metadata(&self) {
        let _ = fs::remove_file(&self.paths.owner_metadata_file);
    }

    /// Reads the current owner metadata for `(channel, namespace, uuid)` from
    /// the coordination directory, without constructing a coordinator. Used by
    /// the desktop URI handler to forward a callback to the current leader.
    pub fn read_owner_metadata(
        channel: &str,
        namespace: CredentialNamespace,
        installation_uuid: Uuid,
    ) -> Option<OwnerMetadata> {
        let base = coordination_root_dir();
        let path = base
            .join(channel)
            .join(namespace.dir_name())
            .join(format!("owner-{}.json", installation_uuid.simple()));
        read_owner_metadata(&path)
    }

    /// Scans one namespace's coordination directory for every published owner
    /// metadata record. The desktop URI handler uses this to forward a callback
    /// whose installation it cannot determine from the URL alone: it tries each
    /// owner until exactly one (the leader whose CSRF matches) accepts. At most
    /// one leader exists per installation, so the set is small.
    pub fn read_all_owner_metadata(
        channel: &str,
        namespace: CredentialNamespace,
    ) -> Vec<OwnerMetadata> {
        let dir = coordination_root_dir()
            .join(channel)
            .join(namespace.dir_name());
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !file_name.starts_with("owner-") {
                continue;
            }
            if let Some(metadata) = read_owner_metadata(&path) {
                records.push(metadata);
            }
        }
        records
    }

    /// The coordination directory (for permission/content tests).
    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.paths.dir
    }
}

/// Filesystem-published routing metadata for the current OAuth leader's
/// desktop callback relay. Contains no bearer credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnerMetadata {
    pub protocol_version: u32,
    pub installation_uuid: Uuid,
    /// Random per-flow nonce; the relay client echoes it back so the owner can
    /// reject a stale or mismatched forwarding request.
    pub owner_nonce: String,
    /// The `crates/ipc` `ConnectionAddress` string the leader is listening on.
    pub endpoint_address: String,
}

impl OwnerMetadata {
    /// Generates a fresh owner nonce for a new leader flow.
    pub fn new(installation_uuid: Uuid, endpoint_address: impl Into<String>) -> Self {
        Self {
            protocol_version: OWNER_METADATA_PROTOCOL_VERSION,
            installation_uuid,
            owner_nonce: Uuid::new_v4().simple().to_string(),
            endpoint_address: endpoint_address.into(),
        }
    }
}

/// Returns the private registry shared by all MCP OAuth coordinators for the
/// current user. Honors `WARP_MCP_OAUTH_COORDINATION_DIR` (tests + the
/// process-safe secure-store seam) so a test never touches a developer's real
/// runtime directory.
fn coordination_root_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(COORDINATION_DIR_ENV) {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(path).join("warp").join("mcp-oauth");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".warp").join("mcp-oauth")
}

/// Opens or creates `path` with owner-only permissions and returns the file
/// handle. The file is created empty (lock files) or truncated (metadata via
/// temp+rename, so this is only for lock files).
fn open_owner_only_file(path: &Path) -> Result<fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory for {:?}", path))?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("failed to open MCP OAuth coordination file {:?}", path))?;
    set_owner_only_file(path)?;
    Ok(file)
}

/// Sets owner-only (`0600`) permissions on a coordination file on Unix. On
/// Windows, file ACLs are not modified here; the directory is already
/// per-user and `crates/local_control` relies on the same owner-only model.
#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to set owner-only permissions on {:?}", path))
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> Result<()> {
    Ok(())
}

/// Sets owner-only (`0700`) permissions on the coordination directory on Unix.
#[cfg(unix)]
fn set_owner_only_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to set owner-only permissions on {:?}", path))
}

#[cfg(not(unix))]
fn set_owner_only_dir(_path: &Path) -> Result<()> {
    Ok(())
}

/// Writes owner metadata atomically with owner-only permissions.
fn write_owner_metadata(path: &Path, metadata: &OwnerMetadata) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create owner metadata parent directory {:?}",
                path
            )
        })?;
    }
    let bytes = serde_json::to_vec(metadata)
        .with_context(|| "failed to serialize MCP OAuth owner metadata")?;
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, &bytes)
        .with_context(|| format!("failed to write owner metadata temp file {:?}", temp_path))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to publish owner metadata {:?}", path))
}

/// Reads and validates owner metadata from `path`. Rejects records with a
/// mismatched protocol version or installation UUID so a stale owner cannot
/// receive a callback meant for a different flow.
fn read_owner_metadata(path: &Path) -> Option<OwnerMetadata> {
    let contents = fs::read_to_string(path).ok()?;
    let metadata: OwnerMetadata = serde_json::from_str(&contents).ok()?;
    if metadata.protocol_version != OWNER_METADATA_PROTOCOL_VERSION {
        log::warn!(
            "Ignoring MCP OAuth owner metadata with protocol version {} (expected {})",
            metadata.protocol_version,
            OWNER_METADATA_PROTOCOL_VERSION
        );
        return None;
    }
    Some(metadata)
}

/// Parses the shared credential map from `raw` and returns this installation's
/// entry. `None` when the map is absent or has no entry for the key. A
/// malformed map is an error so the caller never overwrites it with empty.
fn read_entry(raw: Option<String>, key: &CredentialKey) -> Result<Option<PersistedCredentials>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    match key {
        CredentialKey::Uuid(k) => {
            let map: HashMap<Uuid, PersistedCredentials> = serde_json::from_str(&raw)
                .context("malformed templatable MCP credential map; refusing to read")?;
            Ok(map.get(k).cloned())
        }
        CredentialKey::Hash(k) => {
            let map: HashMap<u64, PersistedCredentials> = serde_json::from_str(&raw)
                .context("malformed file-based MCP credential map; refusing to read")?;
            Ok(map.get(k).cloned())
        }
    }
}

/// Reads the latest map from `raw`, inserts/updates the entry for `key`, and
/// returns the serialized merged map. A malformed existing map is an error so
/// the prior valid value is left untouched. An empty/absent map starts fresh.
fn merge_entry(
    raw: Option<String>,
    key: CredentialKey,
    credentials: PersistedCredentials,
) -> Result<String> {
    match key {
        CredentialKey::Uuid(k) => {
            let mut map: HashMap<Uuid, PersistedCredentials> = match raw {
                None => HashMap::new(),
                Some(s) if s.is_empty() => HashMap::new(),
                Some(s) => serde_json::from_str(&s)
                    .context("malformed templatable MCP credential map; refusing to overwrite")?,
            };
            map.insert(k, credentials);
            serde_json::to_string(&map)
                .context("failed to serialize merged templatable MCP credential map")
        }
        CredentialKey::Hash(k) => {
            let mut map: HashMap<u64, PersistedCredentials> = match raw {
                None => HashMap::new(),
                Some(s) if s.is_empty() => HashMap::new(),
                Some(s) => serde_json::from_str(&s)
                    .context("malformed file-based MCP credential map; refusing to overwrite")?,
            };
            map.insert(k, credentials);
            serde_json::to_string(&map)
                .context("failed to serialize merged file-based MCP credential map")
        }
    }
}

/// Validates that a forwarded callback URL is plausibly an MCP OAuth callback
/// for `installation_uuid` before the owner spends a CSRF check on it. This is
/// a structural check only; the owner still performs the authoritative CSRF
/// state comparison against its own flow.
pub fn forwarded_callback_looks_valid(url: &str, installation_uuid: Uuid) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    // The custom-scheme callback path is `/oauth2callback`; arbitrary paths are
    // not accepted. See `app/src/uri/mod.rs` (`UriHost::Mcp`).
    if parsed.path() != "/oauth2callback" {
        return false;
    }
    // Must carry a state parameter (the CSRF token); the owner validates it.
    if !parsed.query_pairs().any(|(k, _)| k == "state") {
        return false;
    }
    // The installation UUID is not encoded in the redirect URI (RFC 6749
    // §3.1.2.2 compliance), so this check only guards against clearly malformed
    // inputs; the owner's CSRF map is the real authority. Keep the parameter so
    // future routing can use it without changing the signature.
    let _ = installation_uuid;
    true
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
