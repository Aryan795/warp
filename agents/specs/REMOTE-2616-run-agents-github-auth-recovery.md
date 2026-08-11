# REMOTE-2616: Recover `run_agents` after GitHub authentication

## Summary

Keep an accepted `run_agents` action running when a remote child cannot start because GitHub authentication is required. Show one grouped blocker and one server-provided authentication link on the parent card. Retry all eligible retained child requests after the OAuth callback. Complete the original tool action only after every child reaches a terminal launch state.

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
- The confirmation card shows agent-name pills and editable run-wide configuration before acceptance. After acceptance, the same card renders only “Spawning N agents” and then status-only terminal copy in `app/src/ai/blocklist/inline_action/run_agents_card_view.rs:1202-1274,1513-1649`. The top-level cloud-agent launch screen also shows only launch-phase status, not the resolved configuration, in `app/src/terminal/view/ambient_agent/view_impl.rs:874-932`.
- `AIActionStatus::Blocked` is not a generic “waiting on a person” state. `get_action_status` derives it only for the front pending action when no action is running. Accepted asynchronous actions are removed from `pending_actions`, inserted into `running_actions`, and reported as `RunningAsync` in `app/src/ai/blocklist/action_model.rs:428-490,617-641,854-928`.
- Every `AmbientAgentViewModel` subscribes to the singleton `GitHubAuthNotifier`. A single `AuthCompleted` event calls `handle_github_auth_completed` in every model, and every model that is both `NeedsGithubAuth` and retaining an initial request calls `spawn_internal` in `app/src/terminal/view/ambient_agent/model.rs:198-214,1394-1444`. Therefore, completing any one valid GitHub OAuth flow retries all eligible auth-blocked initial children in the process.
- Cancellation only prevents fan-out while plan publication is pending. It does not cancel unresolved child launches after spawning starts in `app/src/ai/blocklist/action_model/execute/run_agents.rs:99-119`.

## Goals

1. Keep the accepted parent action in `AIActionStatus::RunningAsync` while any child is recoverably blocked.
2. Preserve the structured GitHub-auth message and URL from the hidden child to the parent card.
3. Automatically retry each retained initial child request once for each new OAuth completion.
4. Preserve launched siblings while other children launch, block, retry, fail, or are cancelled.
5. Make timeout and cancellation behavior deterministic across OAuth races.
6. Render compact launched/auth-blocked counts with one GitHub CTA. Render named per-child detail only for non-auth failures.

## Non-goals

- Do not use `AIActionStatus::Blocked` for an accepted action. Existing consumers treat it as an unstarted action awaiting tool approval, not an executing action with recoverable progress.
- Do not change the `run_agents` tool schema, server error contract, proto result, or `RunAgentsResult`.
- Do not resume a server run. The blocked create request has no run ID; retry creates the run after authentication.
- Do not retry GitHub-auth-blocked follow-up executions. Existing follow-up behavior remains unchanged.
- Do not persist resumable blocked requests across app restart.
- Do not add post-accept rendering for model, harness, execution mode, environment, worker host, runner, computer use, prompts, auth-secret names, or skills. The accepted request already passed through the existing confirmation UI, and current launch progress surfaces do not repeat resolved configuration.
- Do not render one success or GitHub-auth row per child. Keep the internal slots per child, but group successful and auth-blocked slots in the parent UI.
- Do not add preflight GitHub validation or address environment credential mismatch. Those are separate issues.

## Product behavior

1. After the user accepts `run_agents`, the card keeps the existing compact aggregate presentation. It does not repeat the editable run-wide configuration.
2. While automated launch attempts remain, the card shows `Spawning N agents…` using the existing status-card treatment.
3. After at least one child launches, the card shows `Launched N agents successfully`. It uses singular copy for one agent.
4. While children wait for GitHub authentication, the card shows one `N agents blocked on GitHub auth` group with one `Authenticate with GitHub` button. It uses singular copy for one agent.
5. The grouped auth line uses the public message from the first blocked child in input order and opens that child’s normalized auth URL. The UI does not show child names or one row per auth-blocked child.
6. One successful OAuth return emits the process-wide `AuthCompleted` event. Every eligible auth-blocked initial child retries automatically. The user does not repeat authentication per child.
7. A non-auth failure is the only per-child row. It shows the child name and the existing safe public error. Multiple non-auth failures retain input order.
8. The card emits no tool result while any slot is launching, retrying, or recoverably auth-blocked.
9. The card renders all applicable groups together in this order: launched count, spawning/retrying count, GitHub-auth-blocked count with CTA, then named non-auth failures.
10. When all slots are terminal, the existing result semantics apply:
   - all launched: successful `RunAgentsResult::Launched`;
   - at least one launched: existing successful mixed `RunAgentsResult::Launched`;
   - all permanently failed: existing all-failed `RunAgentsResult::Launched`, which the action model classifies as failed;
   - parent cancellation: `RunAgentsResult::Cancelled`.
11. A terminal card keeps the aggregate launched count and named non-auth failure rows. It does not show an auth group after no slots remain auth-blocked. Successful agent IDs remain available only through the existing child-navigation affordance.
12. A restored in-flight or blocked card renders `Cancelled`. It does not retry after restart.

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

`RunAgentsSpawningSnapshot` becomes a cloneable progress snapshot containing:

- the total, launching, launched, and GitHub-auth-blocked counts;
- the public message and normalized URL from the first auth-blocked child in input order;
- ordered `{ child_name, public_error }` entries for terminal non-auth failures.

Do not copy run-wide configuration or prompt data into the progress snapshot. Emit `RunAgentsExecutorEvent::ProgressUpdated` after every accepted transition.

Process child update streams concurrently. A blocked first slot must not prevent later slots from publishing `Launched`, `Failed`, or `Blocked`.

Record launched children as soon as `Started` arrives. Keep the agent ID in the slot and existing duplicate-launch registry. Build the final `RunAgentsAgentOutcome` vector in input order only after every slot is terminal.

The async action future and result sender stay open while a slot is auth-blocked. The action remains in the action model’s `running_actions` map and therefore stays `AIActionStatus::RunningAsync`.

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

Update `RunAgentsCardView` to extend the existing status-only post-accept card rather than introduce a details surface.

- Keep the current aggregate header, status icon, background, spacing, and terminal labels.
- Show aggregate `Spawning`, `Launched successfully`, and `blocked on GitHub auth` counts.
- Show one auth group and one CTA, sourced from the first blocked child in input order.
- Show named rows only for non-auth failures, using the safe public error already carried by that child outcome.
- Do not show run-wide parameters or per-child prompts after acceptance.
- Use only the already-normalized `CloudAgentStartupBlocker` URL. Accept `https` and the existing Warp return flow. Do not render or log query contents.

The card remains the remediation surface. Do not require the user to discover the hidden child pane.

## Decisions

### Accepted action status

- **Selected:** Keep the parent action `RunningAsync`; model the auth blocker inside the running action’s per-child state.
- `AIActionStatus` is a projection of queue ownership. `Blocked` is returned only for the front of `pending_actions` when the conversation has no `running_actions`. `RunningAsync` is returned for IDs owned by `running_actions` in `app/src/ai/blocklist/action_model.rs:428-490,617-641,854-928`.
- Moving an accepted `run_agents` action back to `pending_actions` would change control flow, not only presentation:
  - the scheduler would treat it as not executed and prevent later serial actions from advancing;
  - `cancel_action_with_id` would use `cancel_pending_action` instead of `cancel_running_async_action`, bypassing `RunAgentsExecutor::cancel_execution` and its retained-child cleanup in `app/src/ai/blocklist/action_model.rs:1069-1125`;
  - the GUI run-agents card would render the editable confirmation card again, while `RunningAsync` renders launch progress in `app/src/ai/blocklist/inline_action/run_agents_card_view.rs:1202-1290`;
  - the TUI run-agents surface would re-enable its acceptance card and register itself as the active blocking input source in `crates/warp_tui/src/orchestration_block/render.rs:230-259`, `crates/warp_tui/src/orchestration_block.rs:463-479`, and `crates/warp_tui/src/agent_block.rs:951-997`.
- Other direct production consumers consistently interpret `Blocked` as approval, not recoverable execution:
  - requested commands and generic TUI tools show approval copy and action buttons instead of running/expandable UI in `app/src/ai/blocklist/inline_action/requested_command.rs:1217-1420`, `crates/warp_tui/src/tui_shell_command_view.rs:410-463`, `crates/warp_tui/src/tui_generic_tool_call_view.rs:345-368`, and `crates/warp_tui/src/tui_file_edits_view.rs:750-807`;
  - search and edit surfaces render a stop/confirmation state or reset to waiting-for-user instead of running progress in `app/src/ai/blocklist/block/view_impl/output.rs:1495-1565,3828-3845`, `app/src/ai/blocklist/inline_action/search_codebase.rs:450-500`, and `app/src/ai/blocklist/inline_action/code_diff_view.rs:603-638`;
  - generic GUI/TUI block routing treats `Blocked` as an input takeover, focus target, and “awaiting approval” label in `app/src/ai/blocklist/block/model/helper.rs:144-160`, `app/src/ai/blocklist/block.rs:4560-4574,4790-4810`, and `crates/warp_tui/src/tool_call_labels.rs:129-190`.
- The canonical transition into `Blocked` also emits `ActionBlockedOnUserConfirmation`. Existing subscribers publish a CLI-agent `permission_request` and mark an active long-running command as waiting on approval in `crates/warp_tui/src/cli_agent_osc_event_publisher.rs:93-137` and `app/src/ai/blocklist/block/cli_controller.rs:223-247`.
- **Rejected:** Set the accepted action to `AIActionStatus::Blocked`. Doing so would require special-casing every consumer above or changing the meaning of a shared queue state. A future distinct “running but awaiting external input” status can be considered separately.

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

### Parent-card density

- **Selected:** Group launched and GitHub-auth-blocked children by count. Show one CTA for the auth group. Keep named rows only for non-auth failures.
- **Why one CTA is sufficient:** `GitHubAuthNotifier` is process-wide, every ambient child subscribes to it, and every eligible blocked initial child retries its retained request from the same callback.
- **Rejected:** One row and one CTA per auth-blocked child. It repeats one provider-level action and becomes noisy for large batches.

### Post-accept configuration

- **Selected:** Match existing launch progress surfaces. Keep the accepted configuration in the pre-accept confirmation card and show only launch status afterward.
- **Rejected:** Add a collapsible resolved-parameter section. Neither the current `run_agents` post-accept card nor the top-level cloud-agent launch screen repeats this configuration, so this issue must not establish a new details pattern.
- **Trade-off:** This narrows the Linear issue’s original “parameters remain inspectable after completion” criterion. The reviewed scope prioritizes GitHub-auth recovery, partial-launch status, and non-auth failure details without adding a new post-launch configuration surface.

## Failure and compatibility behavior

- A repeated GitHub blocker remains recoverable and refreshes its internal metadata. The grouped CTA continues to use the first currently blocked child in input order.
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
2. **Partial success plus blocker:** one child launches while another blocks; the action stays running; the card shows one launched count and one auth-blocked count; recovery produces the ordered all-launched result.
3. **Permanent partial failure plus recovery:** launched, failed, and blocked siblings retain their internal states; the card groups launched/auth-blocked counts and shows only the named non-auth failure row; after recovery the final result preserves current mixed semantics and per-child errors.
4. **Multiple blockers:** multiple children block; the card shows one auth group and one CTA; one callback retries each eligible child once.
5. **Callback before blocker publication:** callback generation advances while the attempt is in flight; the later blocker immediately triggers one retry.
6. **Callback after blocker publication:** the blocked child retries once.
7. **Duplicate callback:** two callbacks during one retry do not create two requests.
8. **Repeated blocker:** a retry that receives another blocker remains blocked until a newer callback.
9. **Timeout pause and re-arm:** the initial timer stops at blocker publication, no timeout fires during human wait, retry gets a fresh 30-second timer, and a stale initial timer is ignored.
10. **Cancel then late callback:** parent cancellation clears the retained request; a later callback cannot spawn; a late task ID is cancelled and cannot complete the parent.
11. **Cancel with launched sibling:** unresolved children cancel; an already-launched sibling is not cancelled.
12. **Restoration:** restoring an in-flight/blocked card renders cancelled and does not retry.
13. **Card content:** active and terminal snapshots render aggregate launch/auth counts, one auth CTA, and ordered named rows only for non-auth failures. Run-wide configuration, prompt, and secret fields do not render after acceptance.
14. **Existing regressions:** top-level initial auth retry, follow-up no-retry, non-auth launch, all-failed result, mixed result, plan-publication cancellation, and terminal-label tests remain green.

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

1. A GitHub-auth-blocked child leaves the parent `run_agents` action in `AIActionStatus::RunningAsync` and produces no tool result.
2. The parent card shows one GitHub-auth-blocked count, the public auth message, and one working `Authenticate with GitHub` link.
3. OAuth callback retry completes the original child slot when the new request receives an agent/run ID.
4. The original `run_agents` action completes only when all slots are terminal.
5. Launched siblings remain launched internally while another child is blocked, and the parent card keeps the launched count visible.
6. Multiple auth-blocked children show one grouped count and one CTA. One OAuth callback retries all eligible blocked initial children.
7. The 30-second timeout runs only during automated attempts and re-arms for retry.
8. Parent cancellation prevents a late callback from spawning an unresolved child.
9. Active and terminal cards show grouped launch/auth counts and named per-child non-auth errors. They do not repeat accepted run-wide parameters.
10. Restored incomplete actions render cancelled and do not retry.
11. The focused tests and repository gates in the test plan pass.
12. The implementation PR includes computer-use video showing `Spawning` → grouped auth blocker with one CTA → OAuth-completed retry → launched count, including a launched sibling whose count remains visible throughout.

## Assumptions and unresolved questions

- **Assumption:** The server-provided GitHub-auth message is public user-facing text. The client still treats the URL query as sensitive and does not log it.
- **Verified current behavior:** One global GitHub OAuth completion is delivered to every `AmbientAgentViewModel` in the app process. Every model that is auth-blocked and retains an initial request retries from that event.
- **Unresolved product questions:** None. The requester approved the parent-card CTA, automatic retained-request retry, and successful completion of the original `run_agents` action.
