//! Prototype `/devcontainer` flow.
//!
//! Unlike the Docker sandbox (which creates its container as a side effect of
//! spawning the PTY), Dev Container lifecycle is explicitly kept off the PTY
//! spawn path: `devcontainer up` can take minutes (image pull, build,
//! `postCreateCommand`), so it runs here, before any pane exists, with a
//! real toast showing progress and a real error toast on failure. Only after
//! `devcontainer up` reports success do we create a pane, using a
//! `ShellStarter::DevContainer` that assumes the container is already
//! running (see `crate::terminal::local_tty::dev_container`).
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use std::collections::HashMap;
#[cfg(feature = "local_tty")]
use std::path::PathBuf;
#[cfg(feature = "local_tty")]
use std::sync::mpsc::SyncSender;

#[cfg(feature = "local_tty")]
use command::r#async::Command;
#[cfg(feature = "local_tty")]
use serde::Deserialize;
#[cfg(feature = "local_tty")]
use warpui::ModelHandle;
use warpui::ViewContext;
#[cfg(feature = "local_tty")]
use warpui::geometry::vector::Vector2F;
#[cfg(not(target_family = "wasm"))]
use warpui::{SingletonEntity, ViewHandle};

use super::TerminalView;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::banner::BannerState;
#[cfg(feature = "local_tty")]
use crate::pane_group::TerminalViewResources;
#[cfg(feature = "local_tty")]
use crate::persistence::ModelEvent;
#[cfg(feature = "local_tty")]
use crate::server::server_api::ServerApiProvider;
#[cfg(feature = "local_tty")]
use crate::terminal::TerminalManager;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::terminal::available_shells::AvailableShell;
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::dev_container::{
    generate_sandbox_id, resolve_devcontainer_cli_path, resolve_docker_cli_path,
};
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::terminal::local_tty::{
    TerminalManager as LocalTtyTerminalManager, TerminalViewSurfaceConfig,
    create_terminal_view_surface,
};
#[cfg(feature = "remote_tty")]
use crate::terminal::remote_tty::TerminalManager as RemoteTtyTerminalManager;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::terminal::shared_session::IsSharedSessionCreator;
#[cfg(feature = "local_tty")]
use crate::view_components::{DismissibleToast, ToastFlavor};
#[cfg(feature = "local_tty")]
use crate::workspace::ToastStack;

/// Object ID shared by every toast in a single `/devcontainer` invocation, so
/// the "building" toast is automatically replaced by the eventual
/// success/error toast instead of stacking.
#[cfg(feature = "local_tty")]
const DEV_CONTAINER_TOAST_OBJECT_ID: &str = "dev-container-build";

/// The last line of `devcontainer up --workspace-folder <dir>` stdout is a
/// single JSON object reporting the outcome. See:
/// <https://github.com/devcontainers/cli>
///
/// On success this also carries what we need to attach with plain `docker
/// exec` afterward (`containerId`/`remoteUser`/`remoteWorkspaceFolder`); see
/// [`crate::terminal::shell::ShellLaunchData::DevContainer`] for why we
/// don't use `devcontainer exec` for that step.
#[cfg(feature = "local_tty")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevContainerUpResult {
    outcome: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    container_id: Option<String>,
    #[serde(default)]
    remote_user: Option<String>,
    #[serde(default)]
    remote_workspace_folder: Option<String>,
}

#[cfg(feature = "local_tty")]
#[allow(unused_variables, clippy::too_many_arguments)]
fn create_dev_container_view(
    resources: TerminalViewResources,
    initial_size: Vector2F,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    #[allow(dead_code)] workspace_folder: PathBuf,
    #[allow(dead_code)] docker_path: PathBuf,
    #[allow(dead_code)] container_id: String,
    #[allow(dead_code)] remote_user: Option<String>,
    #[allow(dead_code)] remote_workspace_folder: String,
    #[allow(dead_code)] sandbox_id: String,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    ViewHandle<TerminalView>,
    ModelHandle<Box<dyn TerminalManager>>,
) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "remote_tty")] {
            let terminal_init = RemoteTtyTerminalManager::create_model(
                resources,
                initial_size,
                model_event_sender,
                ctx.window_id(),
                None, /* initial_input_config */
                ctx,
            );
            let terminal_manager = terminal_init.manager;
            let terminal_view = terminal_init.view;
        } else {
            let user_default_shell_unsupported_banner_model_handle =
                ctx.add_model(|_| BannerState::default());

            let chosen_shell = Some(AvailableShell::new_dev_container_shell(
                workspace_folder,
                docker_path,
                container_id,
                remote_user,
                remote_workspace_folder,
                sandbox_id,
            ));

            let model_event_sender_for_surface = model_event_sender.clone();
            let window_id = ctx.window_id();
            let terminal_init = LocalTtyTerminalManager::<TerminalView>::create_model(
                None,
                HashMap::new(),
                IsSharedSessionCreator::No,
                None, /* restored_blocks */
                user_default_shell_unsupported_banner_model_handle,
                initial_size,
                model_event_sender,
                chosen_shell,
                ctx,
                |surface_init, ctx| {
                    create_terminal_view_surface(
                        TerminalViewSurfaceConfig {
                            resources,
                            model_event_sender: model_event_sender_for_surface,
                            window_id,
                            initial_input_config: None,
                            conversation_restoration: None,
                            has_conversation_restoration: false,
                            is_historical: false,
                            should_use_live_appearance: false,
                            has_restored_command_blocks: false,
                        },
                        surface_init,
                        ctx,
                    )
                },
            );
            let terminal_manager = terminal_init.manager;
            let terminal_view = terminal_init.surface;
        }
    }

    (terminal_view, terminal_manager)
}

impl TerminalView {
    /// Entry point for the `/devcontainer` slash command.
    ///
    /// Finds `.devcontainer/devcontainer.json` for the active session's
    /// directory, resolves the `devcontainer` CLI, and (if both succeed)
    /// hands off to [`Self::bring_up_dev_container`]. Never opens a pane
    /// itself: a pane only appears once the container is confirmed running.
    pub(crate) fn find_and_start_dev_container(&self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_tty")]
        {
            let Some(workspace_folder) =
                self.canonical_session_pwd_if_local(ctx).map(PathBuf::from)
            else {
                self.show_dev_container_toast(
                    "Couldn't determine this session's directory; cd into a local project first."
                        .to_owned(),
                    ToastFlavor::Error,
                    ctx,
                );
                return;
            };

            let devcontainer_config = workspace_folder.join(".devcontainer/devcontainer.json");
            if !devcontainer_config.is_file() {
                self.show_dev_container_toast(
                    format!(
                        "No .devcontainer/devcontainer.json found in {}",
                        workspace_folder.display()
                    ),
                    ToastFlavor::Error,
                    ctx,
                );
                return;
            }

            self.show_dev_container_toast(
                format!(
                    "Building dev container for {}… this can take a few minutes.",
                    workspace_folder.display()
                ),
                ToastFlavor::Default,
                ctx,
            );

            let devcontainer_cli_future = resolve_devcontainer_cli_path(ctx);
            let docker_cli_future = resolve_docker_cli_path(ctx);
            ctx.spawn(
                async move { (devcontainer_cli_future.await, docker_cli_future.await) },
                move |me, (devcontainer_path, docker_path), ctx| {
                    let Some(devcontainer_path) = devcontainer_path else {
                        me.show_dev_container_toast(
                            "devcontainer CLI not found on PATH. Install it with \
                         `npm install -g @devcontainers/cli` and try again."
                                .to_owned(),
                            ToastFlavor::Error,
                            ctx,
                        );
                        return;
                    };
                    let Some(docker_path) = docker_path else {
                        me.show_dev_container_toast(
                            "docker CLI not found on PATH.".to_owned(),
                            ToastFlavor::Error,
                            ctx,
                        );
                        return;
                    };
                    me.bring_up_dev_container(
                        workspace_folder,
                        devcontainer_path,
                        docker_path,
                        ctx,
                    );
                },
            );
        }
        #[cfg(not(feature = "local_tty"))]
        {
            let _ = ctx;
            log::warn!("Dev Container requires the `local_tty` feature; ignoring request");
        }
    }

    /// Runs `devcontainer up` for `workspace_folder`. Only opens a pane once
    /// `up` reports success; shows an error toast (never a pane) otherwise.
    ///
    /// The init script that the eventual `bash --rcfile` session (see
    /// `crate::terminal::local_tty::dev_container`) sources to integrate with
    /// Warp is delivered separately, via `docker cp`, right before that PTY
    /// is spawned — it doesn't need to be wired up here.
    #[cfg(feature = "local_tty")]
    fn bring_up_dev_container(
        &self,
        workspace_folder: PathBuf,
        devcontainer_path: PathBuf,
        docker_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let sandbox_id = generate_sandbox_id();

        let up_future = {
            let devcontainer_path = devcontainer_path.clone();
            let workspace_folder = workspace_folder.clone();
            async move {
                Command::new(&devcontainer_path)
                    .arg("up")
                    .arg("--workspace-folder")
                    .arg(&workspace_folder)
                    .output()
                    .await
            }
        };

        ctx.spawn(up_future, move |me, result, ctx| match result {
            Ok(output) if output.status.success() => {
                match parse_dev_container_up_stdout(&output.stdout) {
                    Some(up_result) if up_result.outcome == "success" => {
                        let (Some(container_id), Some(remote_workspace_folder)) =
                            (up_result.container_id, up_result.remote_workspace_folder)
                        else {
                            me.show_dev_container_toast(
                                "Dev container started, but `devcontainer up` didn't report a \
                                 container ID or workspace folder to attach to."
                                    .to_owned(),
                                ToastFlavor::Error,
                                ctx,
                            );
                            return;
                        };
                        me.show_dev_container_toast(
                            format!(
                                "Dev container ready — opening session in {}…",
                                workspace_folder.display()
                            ),
                            ToastFlavor::Success,
                            ctx,
                        );
                        me.create_and_push_dev_container(
                            workspace_folder,
                            docker_path,
                            container_id,
                            up_result.remote_user,
                            remote_workspace_folder,
                            sandbox_id,
                            ctx,
                        );
                    }
                    _ => {
                        me.show_dev_container_up_failure_toast(&output.stdout, &output.stderr, ctx);
                    }
                }
            }
            Ok(output) => {
                me.show_dev_container_up_failure_toast(&output.stdout, &output.stderr, ctx);
            }
            Err(e) => {
                me.show_dev_container_toast(
                    format!("Failed to run `devcontainer up`: {e}"),
                    ToastFlavor::Error,
                    ctx,
                );
            }
        });
    }

    #[cfg(feature = "local_tty")]
    #[allow(clippy::too_many_arguments)]
    fn create_and_push_dev_container(
        &self,
        workspace_folder: PathBuf,
        docker_path: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_stack) = self
            .pane_stack
            .as_ref()
            .and_then(|stack| stack.upgrade(ctx))
        else {
            log::warn!("Pane stack not available, cannot create dev container session");
            return;
        };

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: ServerApiProvider::as_ref(ctx).get(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let pane_configuration = self.pane_configuration().clone();

        let (terminal_view, terminal_manager) = create_dev_container_view(
            resources,
            self.size_info().pane_size_px(),
            self.model_event_sender.clone(),
            workspace_folder,
            docker_path,
            container_id,
            remote_user,
            remote_workspace_folder,
            sandbox_id,
            ctx,
        );

        terminal_view.update(ctx, |view, _| {
            view.set_pane_configuration(pane_configuration);
        });

        pane_stack.update(ctx, |stack, ctx| {
            stack.push(terminal_manager, terminal_view, ctx);
        });

        ctx.notify();
    }

    #[cfg(feature = "local_tty")]
    fn show_dev_container_toast(
        &self,
        text: String,
        flavor: ToastFlavor,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::new(text, flavor)
                .with_object_id(DEV_CONTAINER_TOAST_OBJECT_ID.to_owned());
            toast_stack.add_persistent_toast(toast, window_id, ctx);
        });
    }

    /// Shows an error toast for a failed `devcontainer up`, preferring the
    /// structured `message`/`description` from its final JSON status line
    /// and falling back to the tail of stderr when that's unavailable.
    #[cfg(feature = "local_tty")]
    fn show_dev_container_up_failure_toast(
        &self,
        stdout: &[u8],
        stderr: &[u8],
        ctx: &mut ViewContext<Self>,
    ) {
        let structured_message = parse_dev_container_up_stdout(stdout).and_then(|result| {
            result
                .message
                .or(result.description)
                .map(|detail| format!("Dev container failed to start: {detail}"))
        });
        let message = structured_message.unwrap_or_else(|| {
            let stderr_text = String::from_utf8_lossy(stderr);
            let tail = tail_lines(&stderr_text, 20);
            format!("Dev container failed to start:\n{tail}")
        });
        self.show_dev_container_toast(message, ToastFlavor::Error, ctx);
    }
}

/// Parses the final JSON status line that `devcontainer up` writes to
/// stdout on completion (both success and failure).
#[cfg(feature = "local_tty")]
fn parse_dev_container_up_stdout(stdout: &[u8]) -> Option<DevContainerUpResult> {
    let stdout_text = String::from_utf8_lossy(stdout);
    let last_line = stdout_text.lines().next_back()?.trim();
    serde_json::from_str(last_line).ok()
}

/// Returns the last `max_lines` non-empty lines of `text`, joined by `\n`.
#[cfg(feature = "local_tty")]
fn tail_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}
