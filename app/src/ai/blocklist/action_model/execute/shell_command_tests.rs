use std::sync::Arc;

use async_channel::unbounded;
use futures::channel::oneshot;
use parking_lot::FairMutex;
use warpui::{App, Entity, EntityId};

use super::{
    AnyActionExecution, BlockSelector, ExecuteActionInput, ShellCommandExecutor,
    ShellCommandExecutorEvent,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::terminal::event::{BlockMetadataReceivedEvent, BlockWorkingDirectoryUpdatedEvent};
use crate::terminal::model::block::{BlockId, BlockMetadata};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::{SessionId, SessionInfo, Sessions};
use crate::terminal::model::terminal_model::{BlockIndex, TerminalModel};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};

/// Locks in the contract that `ShellCommandExecutor`'s requested-command finish
/// detector reacts only to `BlockMetadataReceived` (precmd) and not to
/// `BlockWorkingDirectoryUpdated` (OSC 7). The detector relies on
/// `BlockMetadataReceived` firing exactly once per block; OSC 7 can fire many
/// times per block, so wiring it into the detector would resolve the wait
/// future before the requested command actually finishes.
#[test]
fn block_working_directory_updated_does_not_drain_finish_senders() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_model_events_tx, model_events_rx) = unbounded();
        let model_event_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let active_session = app.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx)
        });
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(
                active_session,
                terminal_model.clone(),
                &model_event_dispatcher,
                terminal_view_id,
                ctx,
            )
        });

        let block_id = BlockId::new();
        let selector = BlockSelector::Id(block_id);
        let (tx, _rx) = oneshot::channel::<()>();
        executor.update(&mut app, |executor, _ctx| {
            executor.block_finished_senders.insert(selector, tx);
        });
        assert_eq!(
            app.read(|ctx| executor.as_ref(ctx).block_finished_senders.len()),
            1
        );

        // OSC 7 update — must NOT drain or resolve the finish sender.
        model_event_dispatcher.update(&mut app, |_dispatcher, ctx| {
            ctx.emit(ModelEvent::BlockWorkingDirectoryUpdated(
                BlockWorkingDirectoryUpdatedEvent {
                    block_metadata: BlockMetadata::new(None, Some("/tmp/new".to_string())),
                    block_index: BlockIndex::zero(),
                    is_for_in_band_command: false,
                    is_done_bootstrapping: true,
                },
            ));
        });
        assert_eq!(
            app.read(|ctx| executor.as_ref(ctx).block_finished_senders.len()),
            1,
            "BlockWorkingDirectoryUpdated must not touch block_finished_senders — \
             that map is reserved for precmd (BlockMetadataReceived)"
        );

        // Precmd event — the senders map should be drained (and since the
        // block isn't in the terminal model, the sender is dropped).
        model_event_dispatcher.update(&mut app, |_dispatcher, ctx| {
            ctx.emit(ModelEvent::BlockMetadataReceived(
                BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(None, Some("/tmp/precmd".to_string())),
                    block_index: BlockIndex::zero(),
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                },
            ));
        });
        assert_eq!(
            app.read(|ctx| executor.as_ref(ctx).block_finished_senders.len()),
            0,
            "BlockMetadataReceived should drain the finish senders"
        );
    });
}

#[derive(Default)]
struct CapturedExecutedCommands(Vec<String>);

impl Entity for CapturedExecutedCommands {
    type Event = ();
}

/// Regression test for a bug where the server always sets `wait_until_completion` to
/// `false` for the modern `run_shell_command` tool call (in both `wait` and `interact`
/// modes), which silently disabled pager decoration even when `uses_pager` was set,
/// letting commands like `gh pr view` or `git log` drop into the user's pager and hang.
#[test]
fn pager_decoration_applies_even_when_wait_until_completion_is_false() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        app.add_singleton_model(|_| RecordingController::new());
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let session_id = SessionId::from(0);
        sessions.update(&mut app, |sessions, _ctx| {
            sessions.register_session_for_test(SessionInfo::new_for_test());
        });

        let (_model_events_tx, model_events_rx) = unbounded();
        let model_event_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        model_event_dispatcher.update(&mut app, |dispatcher, _ctx| {
            dispatcher.set_active_session_id(session_id);
        });

        let active_session = app.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx)
        });
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(
                active_session,
                terminal_model.clone(),
                &model_event_dispatcher,
                terminal_view_id,
                ctx,
            )
        });

        let captured = app.add_model(|_| CapturedExecutedCommands::default());
        captured.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&executor, |captured, _, event, _ctx| {
                if let ShellCommandExecutorEvent::ExecuteCommand { command, .. } = event {
                    captured.0.push(command.clone());
                }
            });
        });

        let action = AIAgentAction {
            id: AIAgentActionId::from("action-1".to_string()),
            task_id: TaskId::new("task-1".to_owned()),
            requires_result: false,
            action: AIAgentActionType::RequestCommandOutput {
                command: "gh pr view 123".to_string(),
                is_read_only: Some(true),
                is_risky: Some(false),
                // The server always reports this as `false` for the modern
                // `run_shell_command` tool, regardless of mode ('wait' or 'interact').
                wait_until_completion: false,
                uses_pager: Some(true),
                rationale: None,
                citations: vec![],
            },
        };
        let conversation_id = AIConversationId::new();

        executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            let _: AnyActionExecution = executor.execute(input, ctx).into();
        });

        let executed_commands = app.read(|ctx| captured.as_ref(ctx).0.clone());
        assert_eq!(executed_commands.len(), 1);
        assert!(
            executed_commands[0].contains("| command cat"),
            "expected pager decoration to be applied even when wait_until_completion is \
             false, got: {}",
            executed_commands[0]
        );
    });
}
