//! Dev Container-specific shell-starter types and helpers.
//!
//! This module owns everything specific to running a Warp shell inside a
//! container that `devcontainer up` (from `@devcontainers/cli`) has already
//! brought up: the [`DevContainerShellStarter`] that carries per-instance
//! state and the host-side init-script mount-point layout.
//!
//! Bringing the container up is *not* this module's concern — that happens
//! before a `DevContainerShellStarter` is ever constructed, driven from
//! `crate::terminal::view::dev_container`, with its own progress/failure UI.
//! This module only knows how to attach a shell to a container that is
//! already running, mirroring [`super::docker_sandbox`]'s split between
//! sandbox lifecycle and the [`super::shell::ShellStarter::DockerSandbox`]
//! variant.

use std::ffi::OsStr;
use std::path::PathBuf;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use warp_core::SessionId;
use warpui::{AppContext, SingletonEntity as _};

use super::shell::DirectShellStarter;
#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;
use crate::terminal::shell::ShellType;
use crate::util::path::{resolve_executable, resolve_executable_in_path};

/// Name of the `@devcontainers/cli` binary we shell out to for `up`.
const DEVCONTAINER_CLI_BIN: &str = "devcontainer";

/// Name of the `docker` CLI binary we shell out to for the interactive
/// attach step. See [`super::shell::ShellStarter::DevContainer`] for why
/// attach goes through plain `docker exec` rather than `devcontainer exec`.
const DOCKER_CLI_BIN: &str = "docker";

/// Resolves a binary using the PATH captured from the user's interactive
/// login shell, matching how `sbx` is resolved for the Docker sandbox (see
/// [`super::docker_sandbox::resolve_sbx_path_from_user_shell`]).
///
/// Falls back to the process's `PATH` if the interactive PATH capture fails.
#[cfg(feature = "local_tty")]
fn resolve_cli_path_from_user_shell(
    bin_name: &'static str,
    ctx: &mut AppContext,
) -> BoxFuture<'static, Option<PathBuf>> {
    let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
        shell_state.get_interactive_path_env_var(ctx)
    });
    async move {
        let path_env_var = path_future.await;
        let resolved = match path_env_var.as_deref() {
            Some(path) => resolve_executable_in_path(bin_name, OsStr::new(path)),
            None => resolve_executable(bin_name),
        };
        resolved.map(|p| p.into_owned())
    }
    .boxed()
}

/// Resolves the `devcontainer` CLI (used for `devcontainer up`).
#[cfg(feature = "local_tty")]
pub fn resolve_devcontainer_cli_path(ctx: &mut AppContext) -> BoxFuture<'static, Option<PathBuf>> {
    resolve_cli_path_from_user_shell(DEVCONTAINER_CLI_BIN, ctx)
}

/// Resolves the `docker` CLI (used for the interactive attach step).
#[cfg(feature = "local_tty")]
pub fn resolve_docker_cli_path(ctx: &mut AppContext) -> BoxFuture<'static, Option<PathBuf>> {
    resolve_cli_path_from_user_shell(DOCKER_CLI_BIN, ctx)
}

/// Root directory on the host under which Dev Container scratch files (bash
/// init scripts) live.
///
/// Lives under the Warp per-user cache directory for the same reasons as
/// [`super::docker_sandbox::docker_sandbox_host_root`]: protected by the
/// user's home-directory permissions, and additionally mode 0700 per
/// sub-directory.
///
/// Layout: `<cache_dir>/dev-container/init/<sandbox_id>/`.
fn dev_container_host_root() -> PathBuf {
    warp_core::paths::cache_dir().join("dev-container")
}

/// Generates a fresh sandbox ID: 8 hex chars (32 bits), plenty for realistic
/// concurrent session counts and keeps paths readable.
///
/// The caller (`crate::terminal::view::dev_container`) must generate this
/// *before* bringing the container up, since the same ID determines the
/// host-side init-script path that gets bind-mounted at `devcontainer up`
/// time — it can't be regenerated later when the [`DevContainerShellStarter`]
/// is constructed, unlike the Docker sandbox's `sandbox_id`, which is
/// generated fresh at PTY-spawn time because sandbox creation and shell
/// attachment happen together there.
pub fn generate_sandbox_id() -> String {
    format!("{:08x}", rand::random::<u32>())
}

/// Host directory where Warp writes a Dev Container session's bash init
/// script, keyed by `sandbox_id`. Mounted read-only into the container at
/// the same absolute path when it is brought up.
pub fn init_dir_for_sandbox_id(sandbox_id: &str) -> PathBuf {
    dev_container_host_root().join("init").join(sandbox_id)
}

/// Full path to a Dev Container session's `init.sh` on the host (also valid
/// inside the container once mounted).
pub fn init_path_for_sandbox_id(sandbox_id: &str) -> PathBuf {
    init_dir_for_sandbox_id(sandbox_id).join("init.sh")
}

/// Wraps a [`DirectShellStarter`] and adds Dev Container-specific parameters.
///
/// Each instance carries a unique `sandbox_id` so multiple Warp panes can
/// attach independent init scripts without colliding on the host-side mount
/// directory, even if they target the same container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevContainerShellStarter {
    pub direct: DirectShellStarter,
    /// Host directory containing `.devcontainer/devcontainer.json`, kept for
    /// display purposes only.
    pub workspace_folder: PathBuf,
    /// Container ID reported by `devcontainer up`, passed to `docker exec`.
    pub container_id: String,
    /// Remote user reported by `devcontainer up` (`docker exec -u`), if any.
    pub remote_user: Option<String>,
    /// Workspace folder inside the container reported by `devcontainer up`
    /// (`docker exec -w`).
    pub remote_workspace_folder: String,
    /// Unique per-instance ID used to derive the host-side init script path.
    /// Generated at construction time; see [`Self::new`].
    pub sandbox_id: String,
    /// The client-generated session ID injected into this container's init script.
    pub session_id: SessionId,
}

impl DevContainerShellStarter {
    /// Construct a new starter for the given `sandbox_id`, which must be the
    /// same ID used to bind-mount the init-script directory when the
    /// container was brought up (see [`generate_sandbox_id`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        direct: DirectShellStarter,
        workspace_folder: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
    ) -> Self {
        let session_id = direct.session_id();
        Self {
            direct,
            workspace_folder,
            container_id,
            remote_user,
            remote_workspace_folder,
            sandbox_id,
            session_id,
        }
    }

    pub fn shell_type(&self) -> ShellType {
        self.direct.shell_type()
    }

    pub fn logical_shell_path(&self) -> &std::path::Path {
        self.direct.logical_shell_path()
    }

    pub fn display_name(&self) -> &str {
        self.direct.display_name()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Host directory where Warp writes this session's bash init script.
    /// Mounted read-only into the container at the same absolute path (the
    /// mount is set up when the container is brought up; see
    /// `crate::terminal::view::dev_container`).
    pub fn init_dir(&self) -> PathBuf {
        init_dir_for_sandbox_id(&self.sandbox_id)
    }

    /// Full path to this session's `init.sh` on the host (also valid inside
    /// the container once mounted).
    pub fn init_path(&self) -> PathBuf {
        init_path_for_sandbox_id(&self.sandbox_id)
    }
}
