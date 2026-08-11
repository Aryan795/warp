# REMOTE-2616: Recover `run_agents` after GitHub authentication

## Summary

Keep an accepted `run_agents` action alive when a remote child cannot start because GitHub authentication is required. Report the action as blocked on the user while it waits, through a new non-terminal action status. Show the blocker and the server-provided authentication link on the parent card. Retry the retained child request after the OAuth callback. Complete the original tool action only after every child reaches a terminal launch state.

References:

- [Linear issue REMOTE-2616](https://linear.app/warpdotdev/issue/REMOTE-2616/show-agent-spawn-parameters-and-per-child-failure-details)
- [Originating Slack thread](https://warpdev.slack.com/archives/C09E37H1NMA/p1786393852525929)
- Code references are pinned to `cd49bd7fe672863e83bbe1ef45491fc30d29e205` on `master`.

## Current state

- The server already returns public GitHub-auth metadata. The client preserves the public message and `auth_url` in `CloudAgentStartupBlocker::GitHubAuthRequired` and converts the URL to the existing Warp OAuth return flow in `app/src/ai/orchestration/remote_child.rs:135-329`.
- `AmbientAgentViewModel` retains the initial `SpawnAgentRequest`. `handle_github_auth_completed` resubmits it after `GitHubAuthEvent::AuthCompleted` in `app/src/terminal/view/ambient_agent/model.rs:128-232,1137-1171,1395-1447`.
- The child-to-parent boundary loses the structured blocker. `AmbientAgentViewModelEvent::NeedsGithubAuth` has no payload, and `TerminalView::handle_ambient_agent_event` writes a static `ConversationStatus::Blocked` string in `app/src/terminal/view/ambient_agent/view_impl.rs:36-38,296-312`.
- `StartAgentExecutor` treats every `ConversationStatus::Blocked` as `StartAgentOutcome::Error`. It removes the pending request before the retried child receives a run ID in `app/src/ai/blocklist/action_model/execute/start_agent.rs:130-220,297-325`.
- `RunAgentsExecutor` consumes one terminal outcome per child. It applies one 30-second timeout around each receiver and exposes only a batch count while spawning in `app/src/ai/blocklist/action_model/execute/run_agents.rs:42-88,191-372`.
- The parent card renders only “Spawning N agents” while active and status-only copy after completion. It does not render child blockers or per-child errors in `app/src/ai/blocklist/inline_action/run_agents_card_view.rs:381-410,1202-1274,1557-1649`. The top-level cloud-agent launch screen also shows only launch-phase status, not resolved configuration, in `app/src/terminal/view/ambient_agent/view_impl.rs:874-932`.
- Cancellation only prevents fan-out while plan publication is pending. It does not cancel unresolved child launches after spawning starts in `app/src/ai/blocklist/action_model/execute/run_agents.rs:99-119`.
- `AIActionStatus` is derived, not stored. `BlocklistAIActionModel::get_action_status` returns `Blocked` only for an action that is still the head of the `pending_actions` queue, and `RunningAsync` for an action that has moved into `running_actions` in `app/src/ai/blocklist/action_model.rs:616-648,859-914`. There is no value that means “started, and cannot continue until the user acts”.
- `AIActionStatus::Blocked` today means exactly one thing: the action has not run and waits for the user to accept or reject it. The canonical transition into it emits `ActionBlockedOnUserConfirmation` in `app/src/ai/blocklist/action_model.rs:809-832`.
- Two matches on `AIActionStatus` are exhaustive with no wildcard arm, so a new variant is a compile error until each decides how to treat it: the GUI status icon in `app/src/ai/blocklist/block/view_impl/output.rs:3834-3858` and the shared TUI tool-call label state in `crates/warp_tui/src/tool_call_labels.rs:142-156`.
- Every `AmbientAgentViewModel` subscribes to the singleton `GitHubAuthNotifier`. A single `AuthCompleted` event calls `handle_github_auth_completed` in every model, and every model that is both `NeedsGithubAuth` and retaining an initial request calls `spawn_internal` in `app/src/terminal/view/ambient_agent/model.rs:198-214,1394-1444`. Therefore, completing any one valid GitHub OAuth flow retries all eligible auth-blocked initial children in the process.

## Goals

1. Keep the accepted parent action alive and non-terminal while any child is recoverably blocked, and report it as blocked on the user rather than as plain running.
2. Make that new status additive: no existing consumer of `AIActionStatus::Blocked` changes behavior, and every exhaustive consumer is forced by the compiler to decide.
3. Preserve the structured GitHub-auth message and URL from the hidden child to the parent card.
4. Automatically retry each retained initial child request once for each new OAuth completion.
5. Preserve launched siblings while other children launch, block, retry, fail, or are cancelled.
6. Make timeout and cancellation behavior deterministic across OAuth races.
7. Summarize child launch progress on the card by state, show one authentication action for the whole batch, and show the public reason for each failed child.

## Non-goals

- Do not reuse the existing `AIActionStatus::Blocked` value for the accepted action. See the “Accepted action status” decision.
- Do not change the parent conversation's `ConversationStatus`. That status feeds notifications, prompt queueing, handoff eligibility, and the tab-close guard; changing what it means is out of scope for this tool call.
- Do not change the `run_agents` tool schema, server error contract, proto result, or `RunAgentsResult`.
- Do not resume a server run. The blocked create request has no run ID; retry creates the run after authentication.
- Do not retry GitHub-auth-blocked follow-up executions. Existing follow-up behavior remains unchanged.
- Do not persist resumable blocked requests across app restart.
- Do not render base prompts, per-child prompt bodies, auth-secret names, or skill contents in the card.
- Do not render the resolved run-wide launch parameters (model, harness, execution mode, environment, worker host, runner, computer use) on the card. See the “Resolved launch parameters” decision.
- Do not render one card row per requested child. See the “Card density” decision.
- Do not add preflight GitHub validation or address environment credential mismatch. Those are separate issues.

## Product behavior

1. After the user accepts `run_agents`, the card summarizes the batch by launch state. It does not list one row per requested child.
2. The active card shows the existing header, then only the state groups that hold at least one child, in this order:
   - `N agents launched`;
   - `N agents waiting for GitHub authentication`, followed by one `Authenticate with GitHub` button;
   - `N agents failed`, followed by one line for each failed child: the child name and its public error.
3. A group that holds one child uses singular copy, for example `1 agent launched`.
4. Children that are still launching or retrying are not given their own group. They are the remainder of the header count, and the header keeps the existing in-progress treatment.
5. The blocked group shows the public server message once, on a single line beneath the group line. When blocked children report different messages, the group shows the message of the first blocked child in input order.
6. All GitHub-auth blockers share one button, because one authentication unblocks the whole batch. A blocker from a different provider, if one is ever added, adds a second button.
7. A launched child stays counted in `N agents launched` while another child waits for authentication.
8. The card emits no tool result while any child is launching, retrying, or recoverably blocked.
9. While any child is recoverably blocked, the parent action reports `AIActionStatus::RunningBlocked`. It returns to `AIActionStatus::RunningAsync` as soon as no child is blocked. The parent conversation's own status does not change.
10. The OAuth return callback automatically retries every eligible blocked initial child. The user authenticates once for the whole batch.
11. When all children are terminal, the existing result semantics apply:
   - all launched: successful `RunAgentsResult::Launched`;
   - at least one launched: existing successful mixed `RunAgentsResult::Launched`;
   - all permanently failed: existing all-failed `RunAgentsResult::Launched`, which the action model classifies as failed;
   - parent cancellation: `RunAgentsResult::Cancelled`.
12. The terminal card keeps the same groups and the existing aggregate label. It shows the launched count and one line for each failed child with its public error. A launched child is reachable only through the existing child navigation affordance; the raw agent ID gets no new text treatment.
13. A restored in-flight or blocked card renders `Cancelled`. It does not retry after restart.

## Technical design

### Per-child state machine

Use one client-only state per input slot:

```rust
pub enum RunAgentsChildLaunchState {
    Launching { attempt: u32 },
    Blocked(StartAgentStartupBlocker),
    Launched { agent_id: String },
    Failed { error: String },
}

pub enum StartAgentStartupBlocker {
    GitHubAuth { message: String, auth_url: String },
}
```

Valid transitions:

```text
Launching(attempt N)
  -> Blocked(GitHubAuth)
  -> Launching(attempt N + 1)
  -> Launched | Failed

Launching(attempt N) -> Launched | Failed
Blocked(GitHubAuth) -> Failed only through cancellation or an explicit terminal condition
Launched | Failed -> no further transition
```

Reject stale or duplicate updates with the child request ID and attempt number. A terminal state is immutable.

### Hidden-child to start-agent contract

Replace the one-shot `StartAgentOutcome` contract with a typed update stream:

```rust
pub enum StartAgentUpdate {
    Launching { attempt: u32 },
    Blocked {
        attempt: u32,
        blocker: StartAgentStartupBlocker,
    },
    Started { agent_id: String },
    Failed { error: String },
}

pub struct StartAgentDispatch {
    pub request_id: StartAgentRequestId,
    pub updates: async_channel::Receiver<StartAgentUpdate>,
}
```

Required behavior:

- `StartAgentExecutor::dispatch` inserts the pending request and returns both its request ID and update receiver.
- The pending entry remains present after `Blocked`.
- `Started` and `Failed` are terminal. Only those updates remove the pending entry.
- `complete_pending_as_started` keeps its current run registration behavior.
- Generic `ConversationStatus::Error` and `Cancelled` still produce `Failed`.
- Generic `ConversationStatus::Blocked` must not independently decide terminal versus recoverable. The GitHub case uses the typed blocker path.
- Extend `AmbientAgentViewModelEvent::NeedsGithubAuth` to carry the public message and URL. Route that payload through the existing terminal/pane child-launch bridge to `StartAgentExecutor::report_blocked(child_conversation_id, blocker)`.
- Emit an explicit retry-attempt update when `handle_github_auth_completed` calls `spawn_internal`. Do not infer retry from display strings.
- The hidden child may continue to use `ConversationStatus::Blocked` for its generic status and pill. The parent launch lifecycle must not reconstruct a blocker from `blocked_action`.
- The blocker and URL are local UI progress data. Do not serialize them into a tool result or model-visible error.

### OAuth idempotency and races

Make `GitHubAuthNotifier` retain a monotonically increasing `completion_generation`.

- Increment the generation before emitting `AuthCompleted`.
- At the start of each launch attempt, store the current generation with that attempt.
- If a callback arrives while a child is already blocked, retry only when its generation is newer than the attempt’s stored generation.
- If a callback arrives after the request starts but before the blocker is published, the blocker handler observes the newer generation and immediately starts exactly one retry.
- Starting the retry records the current generation before issuing the request. If the retry returns the same blocker again without another callback, remain blocked and refresh its message and URL. Do not loop.
- A duplicate callback while a retry is launching is ignored because the slot is not blocked.
- A later callback can retry a repeated blocker once. One callback may retry multiple blocked children.
- An OAuth flow that does not return through Warp leaves the slot blocked. The visible CTA and parent cancellation remain available.

### Run-agents aggregation and progress

Replace `PendingRunAgents::{Publishing, Spawning}` with per-action state that owns:

- one ordered slot per child;
- each `StartAgentRequestId`;
- a cancellation generation/token;
- the terminal result sender.

`RunAgentsSpawningSnapshot` becomes a cloneable progress snapshot containing the ordered child slots. Emit `RunAgentsExecutorEvent::ProgressUpdated` after every accepted transition. The snapshot keeps per-child slots because the executor needs them for ordering, retry, and the final outcome vector; the card groups them for display.

Process child update streams concurrently. A blocked first slot must not prevent later slots from publishing `Launched`, `Failed`, or `Blocked`.

Record launched children as soon as `Started` arrives. Keep the agent ID in the slot and existing duplicate-launch registry. Build the final `RunAgentsAgentOutcome` vector in input order only after every slot is terminal.

The async action future and result sender stay open while a slot is blocked, so the action stays non-terminal.

### Accepted action status

Add a fifth non-terminal variant so an action that has started can report that it cannot continue without the user:

```rust
pub enum AIActionStatus {
    Preprocessing,
    Queued,
    /// The action has not started. It waits on ordering or on the user's confirmation.
    Blocked,
    /// The action started and runs asynchronously.
    RunningAsync,
    /// The action started, and it cannot continue until the user acts.
    RunningBlocked,
    Finished(Arc<AIAgentActionResult>),
}
```

The variant name is not load-bearing; `BlockedWhileRunning` reads equally well. What matters is that it is distinct from `Blocked`.

Helper rules:

- `is_blocked()` keeps its current meaning and matches only `Blocked`. Every existing `is_blocked()` call site therefore keeps its current behavior. This is what makes the change additive.
- `is_running()` matches `RunningAsync` and `RunningBlocked`. An action that waits for authentication is still in flight for phase, scheduling, and cancellation purposes.
- Add `is_blocked_on_user()`, which matches `Blocked` and `RunningBlocked`, for surfaces that want “a human must act” regardless of phase.

Derivation and ownership:

- `BlocklistAIActionModel` holds the set of running action IDs that are blocked on the user. `get_action_status` consults that set before it returns `RunningAsync` (`app/src/ai/blocklist/action_model.rs:616-648`). The queue-based arms are unchanged.
- `RunAgentsExecutor` adds the action to the set when its snapshot gains a first recoverably blocked child, and removes it when no child is blocked or when the action becomes terminal. Nothing else writes the set in this change.
- Do not emit `ActionBlockedOnUserConfirmation` for `RunningBlocked`. That event drives the confirmation flow and the CLI-agent `permission_request` (`crates/warp_tui/src/cli_agent_osc_event_publisher.rs:93-137`, `app/src/ai/blocklist/block/cli_controller.rs:223-247`).

Consumers the compiler will flag, and how each must resolve:

- `app/src/ai/blocklist/block/view_impl/output.rs:3834-3858` (status icon): use the running treatment, not the yellow approval stop glyph.
- `crates/warp_tui/src/tool_call_labels.rs:142-156`: add a label state that reads as waiting on the user. Do not reuse `State::Blocked`, whose copy says “awaiting approval”.
- Any further exhaustive match the compiler reports: default to the `RunningAsync` treatment unless the surface has a reason to distinguish. Record the decision in the implementation PR rather than adding a wildcard arm.

Surfaces that are not compiler-flagged and must still be checked by hand, because they gate on `Blocked` through `matches!` or `is_blocked()`: `run_agents_card_view.rs:1266-1279`, `requested_command.rs:1195-1420`, `code_diff_view.rs:603-638`, `search_codebase.rs:450-500`, `block/model/helper.rs:144-160`, `block.rs:4560-4574`, and the TUI orchestration block listed under “Card changes”. None of them changes behavior under the additive `is_blocked()` rule; the check is to confirm that, not to edit them.

### Timeout policy

Keep the existing 30-second value, but apply it to each automated attempt:

- Arm a timer for `(request_id, attempt)` when `Launching` starts.
- Cancel that timer when the same attempt becomes blocked or terminal.
- Do not run a timer while waiting for human authentication.
- Arm a new 30-second timer when OAuth starts the next attempt.
- A stale timer from an earlier attempt is ignored.
- A current timer expiry calls the same unresolved-start cancellation path used by parent cancellation, then marks that slot `Failed` with the existing timeout message.
- A run ID that arrives after timeout is cancelled/ignored and cannot revise the terminal slot.

### Parent cancellation

Extend `RunAgentsExecutor::cancel_execution` to cover both publication and child-launch phases.

For each nonterminal child:

1. Invalidate the action and child attempt generation.
2. Call `StartAgentExecutor::cancel(request_id)`.
3. Remove the pending entry and close its update stream.
4. Send a child-startup cancellation event through the existing terminal/pane bridge.
5. In `AmbientAgentViewModel`, clear the retained `SpawnAgentRequest` before setting `Cancelled`.
6. If a task ID arrives after cancellation, use the existing server cancellation path.

Do not cancel siblings that already reached `Launched`.

A late OAuth callback, blocker update, timeout, or server token after cancellation must be a no-op except for cancelling a server task that raced with cancellation.

### Card changes

Update `RunAgentsCardView` to render a grouped summary of the progress snapshot rather than a count-only status card.

- Card selection: gate the pre-approval confirmation card on `AIActionStatus::Blocked` only, and the progress card on `RunningAsync` or `RunningBlocked`. This replaces the current `matches!(status, Some(AIActionStatus::Blocked))` check at `run_agents_card_view.rs:1266-1279`, which is the check that would otherwise show Approve/Deny over a launched batch.
- The TUI run-agents surface makes the identical split: keep its acceptance card on `Blocked` only, and do not register itself as the active blocking input source for `RunningBlocked` (`crates/warp_tui/src/orchestration_block/render.rs:230-259`, `crates/warp_tui/src/orchestration_block.rs:463-479`, `crates/warp_tui/src/agent_block.rs:951-997`).
- Header: retain the current aggregate label and status icon.
- Grouping: fold the ordered child slots into counts by state at render time. Emit one line for the launched group and one line for the blocked group. Emit a `N agents failed` line followed by one line per failed child, because each failure carries a different reason that the user needs.
- Group order: launched, blocked, failed. Omit an empty group. Launching and retrying children get no group line.
- CTA area: one button per unique provider, placed on the blocked group line. Every GitHub blocker in the batch shares that button; open the URL of the first blocked child in input order.
- Terminal state: preserve the current aggregate success/mixed/failure labels, then render the same groups beneath them.
- URL handling: use only the already-normalized `CloudAgentStartupBlocker` URL. Accept `https` and the existing Warp return flow. Do not render or log query contents.

Approximate active layout:

```text
Launching 3 agents…
  2 agents launched
  1 agent waiting for GitHub authentication   [Authenticate with GitHub]
```

Approximate terminal layout with a failure:

```text
Launched 2 of 3 agents
  2 agents launched
  1 agent failed
    api-refactor: <public error>
```

The card remains the remediation surface. Do not require the user to discover the hidden child pane.

## Decisions

### Accepted action status

The review's definition of blocked is the one this spec adopts: the action cannot continue until the user acts, so it should say so. The open question was only which value carries it.

- **Selected:** Add a distinct `AIActionStatus::RunningBlocked` and report it for the accepted action while any child waits for authentication.
- **Rejected:** Reuse the existing `AIActionStatus::Blocked`.
- **Rejected:** Keep `RunningAsync` and disambiguate with a flag local to the run-agents card view.
- **Rejected in an earlier revision:** Keep `RunningAsync` and mark the parent conversation `ConversationStatus::Blocked`. Conversation status feeds desktop notifications, prompt queueing, handoff eligibility, and the tab-close guard. Redefining it is too large a semantic change to make in passing for one tool call.

Why not reuse `Blocked`:

- It is not reachable for an accepted action without a structural change anyway. `get_action_status` derives `Blocked` only for an action that is still the head of `pending_actions` while nothing is running for the conversation, and `RunningAsync` once the action has moved into `running_actions` (`app/src/ai/blocklist/action_model.rs:616-648`). Putting the action back on the pending queue to make it derivable would re-run `try_to_execute_action` on the next drain and launch the batch a second time (`app/src/ai/blocklist/action_model.rs:859-914`). Either way, the model needs a new way to express this state; the only choice is whether that new expression reuses an existing value with existing meaning.
- Reusing it silently changes behavior in surfaces that have nothing to do with `run_agents`. Every direct production consumer reads `Blocked` as “this action has not run and is waiting for the user to accept or reject it”:
  - GUI requested-command and generic action renderers show approval copy, stop icons, and accept/reject controls instead of running/expandable UI in `app/src/ai/blocklist/inline_action/requested_command.rs:1217-1420` and `app/src/ai/blocklist/block/view_impl/output.rs:1495-1565,3828-3845`. Several generic render paths require prebuilt action buttons and can panic if an executing action is reported as blocked.
  - GUI search and edit surfaces render confirmation/past-tense state or reset to waiting-for-user instead of running progress in `app/src/ai/blocklist/inline_action/search_codebase.rs:450-500` and `app/src/ai/blocklist/inline_action/code_diff_view.rs:603-638`.
  - GUI block routing treats `Blocked` as user-confirmation input takeover and focus state in `app/src/ai/blocklist/block/model/helper.rs:144-160` and `app/src/ai/blocklist/block.rs:4560-4574,4790-4810`.
  - TUI shell, generic-tool, and file-edit views render permission cards instead of running output in `crates/warp_tui/src/tui_shell_command_view.rs:410-463`, `crates/warp_tui/src/tui_generic_tool_call_view.rs:345-368`, and `crates/warp_tui/src/tui_file_edits_view.rs:750-807`. Shared labels render a stop glyph and append “awaiting approval” in `crates/warp_tui/src/tool_call_labels.rs:129-190`.
  - The TUI run-agents surface re-enables its acceptance card and registers itself as the active blocking input source in `crates/warp_tui/src/orchestration_block/render.rs:230-259`, `crates/warp_tui/src/orchestration_block.rs:463-479`, and `crates/warp_tui/src/agent_block.rs:951-997`.
  - The canonical transition into `Blocked` emits `ActionBlockedOnUserConfirmation`. Subscribers publish a CLI-agent `permission_request` and mark the active long-running command as waiting on approval in `crates/warp_tui/src/cli_agent_osc_event_publisher.rs:93-137` and `app/src/ai/blocklist/block/cli_controller.rs:223-247`.
- Most visibly, the `run_agents` card itself keys off it: `run_agents_card_view.rs:1266-1279` renders the pre-approval confirmation card exactly when the status is `Some(AIActionStatus::Blocked)`. Reporting `Blocked` for an accepted batch would replace the progress card with the Approve/Deny card over agents that have already launched.

Why not a card-local flag:

- The review suggested carrying the distinction inside the run-agents card view. That is the right split, but the wrong place for it. The GUI card is not the only run-agents surface: the TUI orchestration block makes the same acceptance-card and blocking-input decision independently (`crates/warp_tui/src/orchestration_block/render.rs:230-259`, `crates/warp_tui/src/orchestration_block.rs:463-479`, `crates/warp_tui/src/agent_block.rs:951-997`), and the CLI-agent OSC publisher and the shared TUI label state read the status too. A card-local flag would have to be duplicated into each of them, and any surface that forgot would show an approval prompt for an authentication wait.
- Putting the split in the shared enum expresses it once and makes it verifiable. Two matches on `AIActionStatus` are exhaustive, so the compiler enumerates the components that care about action state and refuses to build until each one decides. That is the guarantee the review asked for in the original comment, and no reviewer or author has to remember the list.

What the new variant deliberately does not do:

- It does not notify. The user learns that the batch needs them by looking at the card. Nothing raises a desktop notification or a conversation badge, because that would require the conversation-status change this spec now rejects. See “Open questions”.

### Card density

The review noted that one row per requested child is noisy, and that one authentication unblocks every blocked child, so the user acts once.

- **Selected:** Group children by launch state. Show a count for the launched group and a count plus one shared button for the blocked group. Keep one line per failed child.
- **Rejected:** One row per requested child. It scales badly with a large batch and repeats the same authentication call to action.
- **Rejected:** Group failures into a bare count too. Failures differ from each other, and the reason is the only actionable content; REMOTE-2616 asks for per-child failure detail specifically.

### Resolved launch parameters

- **Selected:** Do not show the resolved model, harness, execution mode, environment, worker host, runner, or computer-use setting on the card.
- **Rejected:** A collapsible resolved-configuration section. The review observed that Warp shows none of these anywhere else once a cloud agent launch is confirmed, so the card would diverge from the rest of the product for no established need. This narrows REMOTE-2616 to the per-child failure half of its title; the spawn-parameters half is dropped rather than deferred.

### Progress transport

- **Selected:** Use a typed nonterminal start-agent update stream.
- **Rejected:** Encode the blocker in `StartAgentOutcome::Error(String)`. It closes the pending request and loses the URL.
- **Rejected:** Parse `ConversationStatus::Blocked.blocked_action`. The string is presentation-only and cannot safely carry a CTA.

### OAuth race control

- **Selected:** Use notifier and attempt generations.
- **Rejected:** React only to the callback event. A callback can arrive before blocker publication and be lost.
- **Rejected:** Retry every blocker after any prior callback. That can create an infinite request loop.

### Server and wire contracts

- **Selected:** Keep this change client-only. The existing server metadata and terminal `RunAgentsResult` are sufficient.
- **Rejected:** Add a server-side blocked run. No server run exists before authentication succeeds.

## Failure and compatibility behavior

- A repeated GitHub blocker remains recoverable and refreshes the CTA metadata.
- A non-GitHub startup failure is terminal and follows current safe user-message handling.
- Partial success keeps current model-visible mixed-result semantics.
- Existing local-child and non-auth remote-child launch behavior remains unchanged.
- Existing top-level initial Cloud Mode auto-retry remains unchanged.
- Existing follow-up Cloud Mode auth behavior remains non-retrying.
- Old stored action results render from `RunAgentsResult` without migration.
- A restored action with no terminal result renders cancelled and never reconstructs a retained request.
- No feature flag or server rollout dependency is required. Ship with the desktop client after the test and visual gates pass.

## Telemetry and observability

Do not add a new product telemetry event in this change. Keep the existing `RunAgentsCompleted` and cloud dispatch events. They must fire once at the final terminal result, not at each blocker transition.

Use test assertions rather than new logging for race correctness. If implementation needs diagnostics, log only request/attempt identifiers and state names. Do not log the auth URL, its query, prompt content, or auth-secret data.

## Test plan

Add focused tests to the existing external test modules for `start_agent`, `run_agents`, ambient-agent model, and card rendering.

1. **All-launched recovery:** child publishes blocker, OAuth completes, retry starts, run ID arrives, original receiver returns launched, and the pending request is removed once.
2. **Partial success plus blocker:** one child launches while another blocks; the action stays running; the launched slot and ID persist; recovery produces the ordered all-launched result.
3. **Permanent partial failure plus recovery:** launched, failed, and blocked siblings retain their states; after recovery the final result preserves current mixed semantics and per-child errors.
4. **Multiple blockers:** several children block; the card shows one blocked group line with the combined count and one button; one callback retries each eligible child once.
5. **Callback before blocker publication:** callback generation advances while the attempt is in flight; the later blocker immediately triggers one retry.
6. **Callback after blocker publication:** the blocked child retries once.
7. **Duplicate callback:** two callbacks during one retry do not create two requests.
8. **Repeated blocker:** a retry that receives another blocker remains blocked until a newer callback.
9. **Timeout pause and re-arm:** the initial timer stops at blocker publication, no timeout fires during human wait, retry gets a fresh 30-second timer, and a stale initial timer is ignored.
10. **Cancel then late callback:** parent cancellation clears the retained request; a later callback cannot spawn; a late task ID is cancelled and cannot complete the parent.
11. **Cancel with launched sibling:** unresolved children cancel; an already-launched sibling is not cancelled.
12. **Restoration:** restoring an in-flight/blocked card renders cancelled and does not retry.
13. **Card content:** active and terminal snapshots render the grouped counts in launched/blocked/failed order, one line per failed child with its public error, and one shared authentication button. Empty groups, prompt bodies, secret names, and run-wide launch parameters do not render.
14. **Card grouping edge cases:** a one-child group uses singular copy; a batch with only launching children shows no group lines; a batch with ten blocked children still shows one group line and one button.
15. **Action status transitions:** `get_action_status` returns `RunningBlocked` on the first blocker, `RunningAsync` once no child is blocked, and `Finished` after the terminal result. `is_blocked()` returns false throughout, `is_running()` returns true throughout, and `is_blocked_on_user()` returns true only while blocked.
16. **Status isolation:** a `RunningBlocked` action does not emit `ActionBlockedOnUserConfirmation`, does not publish a CLI-agent `permission_request`, does not change the parent conversation's `ConversationStatus`, and does not make `AIBlock::is_blocked_on_user_confirmation` true.
17. **Card selection:** the GUI card renders the confirmation card only for `Blocked` and the progress card for both `RunningAsync` and `RunningBlocked`. The TUI orchestration block makes the same split and does not become the active blocking input source for `RunningBlocked`.
18. **Existing regressions:** top-level initial auth retry, follow-up no-retry, non-auth launch, all-failed result, mixed result, plan-publication cancellation, terminal-label, and existing action-status tests remain green.

Run focused tests with:

```text
cargo nextest run -p warp start_agent
cargo nextest run -p warp run_agents
cargo nextest run -p warp github_auth
```

Run repository gates before merge:

```text
./script/format --check
./script/check_no_inline_test_modules
cargo clippy -p warp --all-targets --tests -- -D warnings
```

## Acceptance criteria

1. A GitHub-auth-blocked child leaves the parent `run_agents` action non-terminal, reporting `AIActionStatus::RunningBlocked`, and produces no tool result.
2. The new status is additive: `is_blocked()` stays false for it, no existing `is_blocked()` consumer changes behavior, the parent conversation's `ConversationStatus` is untouched, and no confirmation or permission-request event fires.
3. The parent card shows the blocked count, the public auth message, and a working `Authenticate with GitHub` button, and never shows Approve/Deny for an accepted batch.
4. OAuth callback retry completes the original child slot when the new request receives an agent/run ID.
5. The original `run_agents` action completes only when all slots are terminal.
6. Launched siblings stay counted in the launched group while another child is blocked.
7. Several blocked children collapse to one group line and one shared button.
8. The 30-second timeout runs only during automated attempts and re-arms for retry.
9. Parent cancellation prevents a late callback from spawning an unresolved child.
10. Active and terminal cards show grouped counts and one public error line per failed child, and show no run-wide launch parameters.
11. Restored incomplete actions render cancelled and do not retry.
12. The focused tests and repository gates in the test plan pass.
13. The implementation PR includes computer-use video showing the launched count, the blocked group with its button, the OAuth-completed retry, and the final all-launched card, with a launched sibling counted throughout.

## Assumptions and unresolved questions

- **Assumption:** The server-provided GitHub-auth message is public user-facing text. The client still treats the URL query as sensitive and does not log it.
- **Verified current behavior:** One global GitHub OAuth completion is delivered to every `AmbientAgentViewModel` in the app process. Every model that is auth-blocked and retains an initial request retries from that event.
- **Assumption:** `RunningBlocked` is the right shape for the new status, and the name is negotiable. The requirement is only that it is distinct from `Blocked` so that existing consumers keep their meaning.

### Open questions

- **Nothing tells the user the batch is waiting.** With the conversation-status change dropped, a `RunningBlocked` action produces no badge and no desktop notification. A user who switches tabs after accepting `run_agents` will not learn that authentication is needed until they come back to the card. This is a real gap, and it is out of scope here on purpose: closing it means giving the conversation status a “running but waiting on the user” concept, which is its own change. Worth a follow-up issue; say so and I will file one.
