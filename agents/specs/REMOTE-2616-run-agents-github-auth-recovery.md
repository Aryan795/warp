# REMOTE-2616: Recover `run_agents` after GitHub authentication

## Summary

Keep an accepted `run_agents` action running when a remote child cannot start because GitHub authentication is required. Show the blocker and the server-provided authentication link on the parent card. Retry the retained child request after the OAuth callback. Complete the original tool action only after every child reaches a terminal launch state.

References:

- [Linear issue REMOTE-2616](https://linear.app/warpdotdev/issue/REMOTE-2616/show-agent-spawn-parameters-and-per-child-failure-details)
- [Originating Slack thread](https://warpdev.slack.com/archives/C09E37H1NMA/p1786393852525929)
- Code references are pinned to `aa9f3a4364e66cc49561c69751367240a1035207` on `master`.

## Current state

- The server already returns public GitHub-auth metadata. The client preserves the public message and `auth_url` in `CloudAgentStartupBlocker::GitHubAuthRequired` and converts the URL to the existing Warp OAuth return flow in `app/src/ai/orchestration/remote_child.rs:135-329`.
- `AmbientAgentViewModel` retains the initial `SpawnAgentRequest`. `handle_github_auth_completed` resubmits it after `GitHubAuthEvent::AuthCompleted` in `app/src/terminal/view/ambient_agent/model.rs:128-232,1137-1171,1395-1447`.
- The child-to-parent boundary loses the structured blocker. `AmbientAgentViewModelEvent::NeedsGithubAuth` has no payload, and `TerminalView::handle_ambient_agent_event` writes a static `ConversationStatus::Blocked` string in `app/src/terminal/view/ambient_agent/view_impl.rs:36-38,296-312`.
- `StartAgentExecutor` treats every `ConversationStatus::Blocked` as `StartAgentOutcome::Error`. It removes the pending request before the retried child receives a run ID in `app/src/ai/blocklist/action_model/execute/start_agent.rs:130-220,297-325`.
- `RunAgentsExecutor` consumes one terminal outcome per child. It applies one 30-second timeout around each receiver and exposes only a batch count while spawning in `app/src/ai/blocklist/action_model/execute/run_agents.rs:42-88,191-372`.
- The parent card renders only “Spawning N agents” while active and status-only copy after completion. It does not render child blockers, per-child errors, or resolved parameters in `app/src/ai/blocklist/inline_action/run_agents_card_view.rs:381-410,1202-1274,1557-1649`.
- Cancellation only prevents fan-out while plan publication is pending. It does not cancel unresolved child launches after spawning starts in `app/src/ai/blocklist/action_model/execute/run_agents.rs:99-119`.

## Goals

1. Keep the accepted parent action in `AIActionStatus::RunningAsync` while any child is recoverably blocked.
2. Preserve the structured GitHub-auth message and URL from the hidden child to the parent card.
3. Automatically retry each retained initial child request once for each new OAuth completion.
4. Preserve launched siblings while other children launch, block, retry, fail, or are cancelled.
5. Make timeout and cancellation behavior deterministic across OAuth races.
6. Render requested children, per-child state, per-child public errors, and the resolved run-wide launch parameters.

## Non-goals

- Do not use `AIActionStatus::Blocked` for an accepted action. That status remains pre-execution or awaiting user confirmation.
- Do not change the `run_agents` tool schema, server error contract, proto result, or `RunAgentsResult`.
- Do not resume a server run. The blocked create request has no run ID; retry creates the run after authentication.
- Do not retry GitHub-auth-blocked follow-up executions. Existing follow-up behavior remains unchanged.
- Do not persist resumable blocked requests across app restart.
- Do not render base prompts, per-child prompt bodies, auth-secret names, or skill contents in the card.
- Do not add preflight GitHub validation or address environment credential mismatch. Those are separate issues.

## Product behavior

1. After the user accepts `run_agents`, the card shows one ordered row for every `agent_run_configs[]` entry.
2. A row moves through `Starting` → `Waiting for GitHub authentication` → `Retrying` → `Started` or `Failed`.
3. A blocked row shows the public server message. The card shows an `Authenticate with GitHub` button that opens the blocker URL.
4. Identical GitHub provider actions are deduplicated to one button. Child rows remain separate.
5. A launched sibling stays shown as `Started` while another row waits for authentication.
6. The card emits no tool result while any row is `Starting`, `Retrying`, or recoverably blocked.
7. The OAuth return callback automatically retries every eligible blocked initial child.
8. When all rows are terminal, the existing result semantics apply:
   - all launched: successful `RunAgentsResult::Launched`;
   - at least one launched: existing successful mixed `RunAgentsResult::Launched`;
   - all permanently failed: existing all-failed `RunAgentsResult::Launched`, which the action model classifies as failed;
   - parent cancellation: `RunAgentsResult::Cancelled`.
9. The active and terminal cards expose the accepted run-wide model, harness, execution mode, and remote environment, worker host, runner, and computer-use setting. Empty optional values display as “Default”.
10. A terminal card keeps the same child rows. Failed rows show the public per-child error. Successful rows show the agent ID only through the existing child navigation affordance; the raw ID does not need a new text treatment.
11. A restored in-flight or blocked card renders `Cancelled`. It does not retry after restart.

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

- the accepted, resolved request snapshot;
- one ordered slot per child;
- each `StartAgentRequestId`;
- a cancellation generation/token;
- the terminal result sender.

`RunAgentsSpawningSnapshot` becomes a cloneable progress snapshot containing the resolved run-wide display fields and ordered child rows. Emit `RunAgentsExecutorEvent::ProgressUpdated` after every accepted transition.

Process child update streams concurrently. A blocked first slot must not prevent later slots from publishing `Launched`, `Failed`, or `Blocked`.

Record launched children as soon as `Started` arrives. Keep the agent ID in the slot and existing duplicate-launch registry. Build the final `RunAgentsAgentOutcome` vector in input order only after every slot is terminal.

The async action future and result sender stay open while a slot is blocked. This keeps the existing action in `AIActionStatus::RunningAsync`.

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

Update `RunAgentsCardView` to render the progress snapshot rather than a count-only status card.

- Header: retain the current aggregate label and status icon.
- Child rows: name, state, and public failure/blocker detail.
- CTA area: one button per unique `(provider, auth_url)` pair, in first-child order.
- Details: a compact, collapsible resolved-configuration section. Show model, harness, local/remote, environment, worker host, runner, and computer use. Reuse the same layout for terminal results.
- Terminal state: preserve the current aggregate success/mixed/failure labels, then render rows and details beneath them.
- URL handling: use only the already-normalized `CloudAgentStartupBlocker` URL. Accept `https` and the existing Warp return flow. Do not render or log query contents.

The card remains the remediation surface. Do not require the user to discover the hidden child pane.

## Decisions

### Accepted action status

- **Selected:** Keep the parent action `RunningAsync`; model the blocker per child.
- **Rejected:** Set the parent to `AIActionStatus::Blocked`. That status means the action has not begun and is waiting on ordering or confirmation.

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
2. **Partial success plus blocker:** one child launches while another blocks; the action stays running; the launched row and ID persist; recovery produces the ordered all-launched result.
3. **Permanent partial failure plus recovery:** launched, failed, and blocked siblings retain their states; after recovery the final result preserves current mixed semantics and per-child errors.
4. **Multiple blockers:** multiple rows block; identical GitHub CTAs deduplicate; one callback retries each eligible child once.
5. **Callback before blocker publication:** callback generation advances while the attempt is in flight; the later blocker immediately triggers one retry.
6. **Callback after blocker publication:** the blocked child retries once.
7. **Duplicate callback:** two callbacks during one retry do not create two requests.
8. **Repeated blocker:** a retry that receives another blocker remains blocked until a newer callback.
9. **Timeout pause and re-arm:** the initial timer stops at blocker publication, no timeout fires during human wait, retry gets a fresh 30-second timer, and a stale initial timer is ignored.
10. **Cancel then late callback:** parent cancellation clears the retained request; a later callback cannot spawn; a late task ID is cancelled and cannot complete the parent.
11. **Cancel with launched sibling:** unresolved children cancel; an already-launched sibling is not cancelled.
12. **Restoration:** restoring an in-flight/blocked card renders cancelled and does not retry.
13. **Card content:** active and terminal snapshots render ordered rows, public errors, deduplicated CTA, and resolved run-wide parameters. Prompt and secret fields do not render.
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
2. The parent card shows the affected child, public auth message, and working `Authenticate with GitHub` link.
3. OAuth callback retry completes the original child slot when the new request receives an agent/run ID.
4. The original `run_agents` action completes only when all slots are terminal.
5. Launched siblings remain launched and visible while another child is blocked.
6. Multiple blocked children show separate rows and deduplicated provider CTAs.
7. The 30-second timeout runs only during automated attempts and re-arms for retry.
8. Parent cancellation prevents a late callback from spawning an unresolved child.
9. Active and terminal cards show per-child states/errors and accepted run-wide parameters.
10. Restored incomplete actions render cancelled and do not retry.
11. The focused tests and repository gates in the test plan pass.
12. The implementation PR includes computer-use video showing `Starting` → auth blocker with CTA → OAuth-completed retry → `Started`, including a launched sibling that remains visible throughout. The video also shows the resolved parameter details.

## Assumptions and unresolved questions

- **Assumption:** The server-provided GitHub-auth message is public user-facing text. The client still treats the URL query as sensitive and does not log it.
- **Assumption:** One global GitHub OAuth completion is valid for every blocked GitHub child in the current app process.
- **Unresolved product questions:** None. The requester approved the parent-card CTA, automatic retained-request retry, and successful completion of the original `run_agents` action.
