use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{FairMutex, Mutex};
use warp_core::SessionId;
use warpui::App;

use super::*;
use crate::terminal::ShellLaunchData;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::StartCommandOutcome;
use crate::terminal::model::ansi::{Handler, PreexecValue};
use crate::terminal::model::session::{
    BootstrapSessionType, HostInfo, IsSSHWrapperSession, Sessions,
};
use crate::terminal::shell::Shell;

#[derive(Clone, Default)]
struct TestEventLoopSender {
    messages: Arc<Mutex<Vec<Message>>>,
}

impl EventLoopSender for TestEventLoopSender {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError> {
        self.messages.lock().push(message);
        Ok(())
    }
}

fn terminal_model() -> Arc<FairMutex<TerminalModel>> {
    Arc::new(FairMutex::new(TerminalModel::mock(
        None,
        Some(ChannelEventListener::new_for_test()),
    )))
}

fn session_info_with_launch_data(launch_data: Option<ShellLaunchData>) -> SessionInfo {
    SessionInfo {
        session_id: SessionId::from(1),
        shell: Shell::new(ShellType::Bash, None, None, HashSet::new(), None),
        launch_data,
        histfile: None,
        user: "test-user".to_owned(),
        hostname: "test-host".to_owned(),
        subshell_info: None,
        path: None,
        environment_variable_names: HashSet::new(),
        aliases: HashMap::new(),
        abbreviations: HashMap::new(),
        function_names: HashSet::new(),
        builtins: HashSet::new(),
        keywords: Vec::new(),
        is_ssh_wrapper_session: IsSSHWrapperSession::No,
        home_dir: None,
        cdpath: None,
        editor: None,
        session_type: BootstrapSessionType::Local,
        host_info: HostInfo::default(),
        wsl_name: None,
        spawning_session_id: None,
    }
}

/// Sets up a `PtyController` with a fake event loop sender, returning both so
/// tests can drive `initialize_shell` and inspect what it sent.
fn controller_with_test_sender(
    app: &mut App,
) -> (
    ModelHandle<PtyController<TestEventLoopSender>>,
    TestEventLoopSender,
) {
    let model = terminal_model();
    let (_model_events_tx, model_events_rx) = async_channel::unbounded();
    let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let model_events =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let line_editor_status =
        app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
    let sender = TestEventLoopSender::default();
    let controller = app.add_model(|ctx| {
        PtyController::new(
            sender.clone(),
            model_events,
            line_editor_status,
            sessions,
            executor_command_rx,
            model,
            ctx,
        )
    });
    (controller, sender)
}

#[test]
fn initialize_shell_is_noop_for_dev_container_session() {
    App::test((), |mut app| async move {
        let (controller, sender) = controller_with_test_sender(&mut app);
        let session_info = session_info_with_launch_data(Some(ShellLaunchData::DevContainer {
            workspace_folder: "/home/user/project".into(),
            docker_path: "/usr/bin/docker".into(),
            container_id: "abc123".to_owned(),
            remote_user: None,
            remote_workspace_folder: "/workspaces/project".to_owned(),
            sandbox_id: "deadbeef".to_owned(),
            session_id: SessionId::from(1),
        }));

        controller.update(&mut app, |controller, ctx| {
            controller.initialize_shell(&session_info, ctx);
        });

        assert!(
            sender.messages.lock().is_empty(),
            "Dev Container sessions bootstrap from files already staged into the container, so \
             Warp must not also type the bootstrap script into the pty."
        );
    });
}

#[test]
fn initialize_shell_writes_bootstrap_bytes_for_local_session() {
    App::test((), |mut app| async move {
        let (controller, sender) = controller_with_test_sender(&mut app);
        let session_info = session_info_with_launch_data(Some(ShellLaunchData::Executable {
            executable_path: "/bin/bash".into(),
            shell_type: ShellType::Bash,
        }));

        controller.update(&mut app, |controller, ctx| {
            controller.initialize_shell(&session_info, ctx);
        });

        assert!(
            sender
                .messages
                .lock()
                .iter()
                .any(|message| matches!(message, Message::Input(_))),
            "A local (non-Dev-Container) session should still have its bootstrap script written \
             to the pty."
        );
    });
}

#[test]
fn rejected_and_coalesced_starts_do_not_mutate_controller_or_write_bytes() {
    App::test((), |mut app| async move {
        let model = terminal_model();
        let (model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status,
                sessions,
                executor_command_rx,
                model.clone(),
                ctx,
            )
        });
        controller.update(&mut app, |controller, _| {
            controller.pending_writes.push_back(PtyWrite::Bytes {
                bytes: b"existing-pending-write".to_vec().into(),
            });
        });

        assert_eq!(
            model.lock().start_command_execution(),
            StartCommandOutcome::Accepted
        );
        let coalesced = controller.update(&mut app, |controller, ctx| {
            controller.write_command(
                "coalesced",
                ShellType::Zsh,
                CommandExecutionSource::User,
                ctx,
            )
        });
        assert_eq!(coalesced, StartCommandOutcome::Coalesced);
        controller.read(&app, |controller, _| {
            assert!(!controller.is_user_command_executing);
            assert_eq!(controller.pending_writes.len(), 1);
        });
        assert!(sender.messages.lock().is_empty());

        model.lock().preexec(PreexecValue {
            command: "running".to_owned(),
            session_id: None,
        });
        let rejected = controller.update(&mut app, |controller, ctx| {
            controller.write_command(
                "rejected",
                ShellType::Zsh,
                CommandExecutionSource::User,
                ctx,
            )
        });
        assert_eq!(rejected, StartCommandOutcome::RejectedExecuting);
        controller.read(&app, |controller, _| {
            assert!(!controller.is_user_command_executing);
            assert_eq!(controller.pending_writes.len(), 1);
        });
        assert!(sender.messages.lock().is_empty());

        drop(model_events_tx);
    });
}

#[test]
fn rejected_queued_in_band_start_is_cancelled_without_writing_bytes() {
    App::test((), |mut app| async move {
        let model = terminal_model();
        model.lock().start_command_execution();
        model.lock().preexec(PreexecValue {
            command: "running".to_owned(),
            session_id: None,
        });

        let (model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status.clone(),
                sessions,
                executor_command_rx,
                model.clone(),
                ctx,
            )
        });
        let (cancel_tx, cancel_rx) = async_channel::unbounded();

        controller.update(&mut app, |controller, ctx| {
            controller.queue_in_band_command(
                "rejected-in-band",
                ShellType::Zsh,
                "command-id".to_owned(),
                cancel_tx,
                ctx,
            );
            let write = controller
                .pending_writes
                .pop_front()
                .expect("The inactive line editor should leave the in-band command queued.");
            assert!(!controller.send_write_to_event_loop(write, ctx));
        });

        assert_eq!(
            cancel_rx
                .try_recv()
                .expect("The rejected in-band command should be cancelled.")
                .command_id,
            "command-id"
        );
        assert!(sender.messages.lock().is_empty());
        line_editor_status.read(&app, |line_editor_status, _| {
            assert!(!line_editor_status.is_line_editor_active());
        });
        drop(model_events_tx);
    });
}

/// Regression test for the input-reporting probe being echoed instead of consumed by the shell:
/// a write queued behind the probe must not be sent in the same synchronous tick, since a relay
/// slow enough to separate the probe's `write()` from the shell consuming it could otherwise let
/// the two land in the same read.
#[test]
fn queued_write_behind_probe_is_deferred_to_a_separate_tick() {
    App::test((), |mut app| async move {
        let session_id = SessionId::from(7);
        let session_info = SessionInfo {
            shell: Shell::new(ShellType::Zsh, None, None, HashSet::new(), None),
            session_id,
            ..session_info_with_launch_data(None)
        };

        let model = terminal_model();
        let (_model_events_tx, model_events_rx) = async_channel::unbounded();
        let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| {
            let mut sessions = Sessions::new_for_test();
            sessions.register_session_for_test(session_info);
            sessions
        });
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        model_events.update(&mut app, |dispatcher, _| {
            dispatcher.set_active_session_id(session_id);
        });
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
        let sender = TestEventLoopSender::default();
        let controller = app.add_model(|ctx| {
            PtyController::new(
                sender.clone(),
                model_events,
                line_editor_status.clone(),
                sessions,
                executor_command_rx,
                model,
                ctx,
            )
        });

        controller.update(&mut app, |controller, _| {
            controller.pending_writes.push_back(PtyWrite::Bytes {
                bytes: b"queued-write".to_vec().into(),
            });
        });

        line_editor_status.update(&mut app, |line_editor_status, ctx| {
            line_editor_status.set_active_for_test(ctx);
        });

        {
            let messages = sender.messages.lock();
            assert_eq!(
                messages.len(),
                1,
                "only the input-reporting probe should be sent in the same tick as Active"
            );
            assert!(matches!(
                &messages[0],
                Message::Input(bytes) if bytes[..] == [escape_sequences::C0::ESC, b'i']
            ));
        }

        warpui::r#async::Timer::after(PENDING_WRITE_SETTLE_DELAY * 4).await;

        let messages = sender.messages.lock();
        assert_eq!(
            messages.len(),
            2,
            "the write queued behind the probe should be sent once it has settled"
        );
        assert!(matches!(
            &messages[1],
            Message::Input(bytes) if bytes[..] == *b"queued-write"
        ));
    });
}
