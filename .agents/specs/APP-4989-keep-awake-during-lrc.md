# APP-4989: keep the computer awake during agent long-running command execution

## PRODUCT

**Summary:** The Warp client already has keep-awake logic (`crates/prevent_sleep`), but
today a wake assertion is held only for the lifetime of a single multi-agent SSE
request (one turn). It is dropped at `StreamFinished`, so the machine can idle-sleep
while the client executes tool calls locally between turns — most importantly while a
long-running shell command runs across turns. This change closes that gap by holding a
wake assertion continuously across the whole in-progress agent conversation — including
the local command-execution window between `StreamFinished` and the next follow-up turn
(`WriteToLongRunningShellCommand` / `ReadShellCommandOutput` polling) — and releasing it
promptly when the conversation reaches a terminal state, is cancelled, or goes idle
waiting for user input. The change is client-side in `warpdotdev/warp`; no server change
is required. Per the requester's explicit direction, this first change is scoped to the
long-running command (LRC) case and to *system* sleep only.

**User-visible invariants:**

1. While an in-app agent conversation is running (status `InProgress`), a system
   idle-sleep wake assertion is held continuously — across every turn and across the
   local command-execution gap between turns — so the machine does not idle-sleep while
   the agent runs a long-running local command (e.g. `sleep 600`).
2. The assertion is held across transient failures (`TransientError`, while an automatic
   retry/resume is pending) so recovery is not interrupted by sleep.
3. The assertion is released promptly when the conversation stops running: on a terminal
   status (`Success`, `Error`, `Cancelled`), when the agent yields to wait for events or
   user input (`WaitingForEvents`), or when an action is blocked pending user approval
   (`Blocked`). It is re-acquired if the conversation resumes (returns to `InProgress`).
4. The assertion is released when the conversation is cleaned up (pane cleared,
   conversation removed/deleted) so closing the pane or deleting the conversation never
   leaves a stray wake assertion.
5. The existing per-request assertion (held during each SSE stream) is unchanged and
   stacks cleanly with the new conversation-level assertion — no double-acquire panic,
   no leak, no change to streaming-turn behavior.
6. Auto-handoff-on-sleep behavior is unchanged: the existing long-running-command skip
   (`AutoCloudHandoffSkipReason::LongRunningCommand`) is not altered, and no new setting
   or flag is introduced that affects handoff gating. Covering the LRC gap here is
   complementary to handoff (which already skips LRCs).
7. On Linux the keep-awake backend remains a no-op (out of scope for this change); the
   new model still compiles and runs on Linux and on wasm without changing behavior
   (wasm guard is a no-op).

**Key design choices:** Scope the wake assertion to the agent *conversation* lifecycle
(held across turns, including the local command-execution gap), not per-request. Manage
the guard in one small dedicated model keyed by conversation id, subscribed to
`BlocklistAIHistoryModel` status + cleanup events, so the acquire/release logic is
centralized and unit-testable. Keep the existing per-request guard in place as defense
in depth (removing it is a non-goal). Use a distinct reason string
(`"Agent Mode run in-progress"`) from the per-request `"Agent Mode request in-progress"`.

## TECH

**Current context (all references pinned to `fb22b1920c59bc6e6aa969d81c797ec24e7b1830`, current `master` HEAD):**

- `crates/prevent_sleep/src/lib.rs:17` exposes `prevent_sleep::prevent_sleep(reason: &'static str) -> Guard`;
  the guard releases the assertion on `Drop`. `Guard` is `Send + Sync` on macOS
  (`crates/prevent_sleep/src/mac.rs:16-17`) and sendable on Windows (the guard holds an
  `mpsc::Sender`, `crates/prevent_sleep/src/windows.rs:143-146`). `Stream::wrap`
  (`lib.rs:33`) wraps a stream with an optional guard.
- `crates/prevent_sleep/src/mac.rs:34` uses `NSActivityOptions::UserInitiated` (includes
  `IdleSystemSleepDisabled`, prevents idle *system* sleep; does **not** include
  `IdleDisplaySleepDisabled`). `crates/prevent_sleep/src/windows.rs:59-63` uses
  `ES_CONTINUOUS | ES_AWAYMODE_REQUIRED | ES_SYSTEM_REQUIRED` (no `ES_DISPLAY_REQUIRED`).
  `crates/prevent_sleep/src/noop.rs` is a no-op `Guard` for Linux/other/wasm
  (`crates/prevent_sleep/build.rs:6`: `noop: { not(any(macos, windows)) }`).
- The only callers that request a guard are in `crates/http_client/src/lib.rs`:
  `execute_inner` (`lib.rs:381`, non-streaming round-trip) and `eventsource()`
  (`lib.rs:553-557`, wraps the SSE stream via `prevent_sleep::Stream::wrap`). The only
  place that *requests* one is `crates/warp_multi_agent_client/src/lib.rs:59`:
  `.prevent_sleep("Agent Mode request in-progress")` on the multi-agent SSE request.
  `prevent_sleep` is a workspace dependency (`Cargo.toml:65`) currently used only by
  `http_client` (`crates/http_client/Cargo.toml:21`); the `app` crate does **not**
  currently depend on it.
- The agent runs as a sequence of turns. Each turn = one SSE request = one
  `ResponseStream` = one per-request guard. `app/src/ai/blocklist/controller.rs:2490-2505`
  constructs `ResponseStream::new(...)` per turn; `controller.rs:2542-2547` sets the
  conversation to `InProgress` when a request is sent. After a turn's stream finishes,
  `controller.rs:3035+` (`AfterStreamFinished`) queues the turn's actions for local
  execution, and `controller.rs:1572-1667` (`send_follow_up_for_conversation`) sends a
  *new* turn (new stream / new guard) once actions complete. The conversation stays
  `InProgress` during local action execution between turns.
- `app/src/ai/blocklist/action_model/execute/shell_command.rs:38-49` (`ShellCommandExecutor`)
  acquires **no** `prevent_sleep` guard. Its poll future (`action_result_future`,
  `shell_command.rs:513`) waits on timers up to `MAX_AGENT_DELAY_DURATION` (120s,
  `shell_command.rs:56`) or `MAX_WAIT_DURATION` (2s, `:52`) and on block-completion
  signals. `ReadShellCommandOutput` (`:349-389`) and `WriteToLongRunningShellCommand`
  (`:285-348`) poll across turns, so the inter-turn gap can be many minutes and recur
  many times for one command. `supports_long_running_commands: true` is advertised at
  `app/src/ai/agent/api/impl.rs:82`.
- `app/src/ai/agent/conversation.rs:4589-4613` defines `ConversationStatus`:
  `InProgress`, `Success`, `Error`, `TransientError`, `Cancelled`, `Blocked { .. }`,
  `WaitingForEvents`. `is_done()` (`:4713-4718`) covers `Success`/`Error`/`Cancelled`;
  `is_waiting_for_events()` (`:4722-4724`) covers `WaitingForEvents`.
  `update_status_with_error` (`conversation.rs:973-997`) emits
  `BlocklistAIHistoryEvent::UpdatedConversationStatus { conversation_id, terminal_surface_id, update: ConversationStatusUpdate::Changed { prev_status }, new_status }` on every status set.
- `app/src/ai/blocklist/history_model.rs:2938-2945` defines
  `BlocklistAIHistoryEvent::UpdatedConversationStatus`; `:2883-2886` defines
  `ConversationStatusUpdate { Restored, Changed { prev_status } }` (`Restored` is emitted
  on conversation re-load, `history_model.rs:1149`). Cleanup events:
  `ClearedConversationsForTerminalSurface` (`:2959-2965`, carries `cleared_conversation_ids`),
  `RemoveConversation` (`:2987-2991`), `DeletedConversation` (`:2993-3001`).
- `app/src/workspace/auto_handoff.rs:198-200` handles `CpuWillSleep`; `:328-330` skips
  handoff with `AutoCloudHandoffSkipReason::LongRunningCommand` when
  `has_active_long_running_command()`. `app/src/settings/ai.rs:1930-1939`:
  `auto_handoff_on_sleep_enabled` defaults to `false`, macOS only.

**Trace/reproduction evidence:** The gap is structural and confirmed by call-graph
tracing (carried forward from the APP-4989 triage findings): `prevent_sleep` is depended
on only by `http_client`; the only guard request site is the multi-agent SSE request;
`ShellCommandExecutor` and the terminal command path acquire no guard; each turn is a
separate `ResponseStream`/SSE stream. Manual corroboration (macOS): run an agent turn
that executes a long-running local command (e.g. ask the agent to run `sleep 600`);
while the command is running, `pmset -g assertions` shows **no** `PreventUserIdleSystemSleep`
assertion from Warp during the inter-turn gap (the assertion appears only while an SSE
turn is actively streaming), and the machine will idle-sleep during the command. After
this change the assertion must be present continuously through the `sleep 600` run.

### Proposed changes

1. **Add a conversation-scoped wake-assertion model.**
   - Add a new model (e.g. `AgentRunSleepGuardModel`) that owns a
     `HashMap<AIConversationId, prevent_sleep::Guard>` and subscribes to
     `BlocklistAIHistoryModel` events. Register it in the app model graph alongside the
     other `BlocklistAI*` singletons/models, on both native and wasm targets.
   - On `UpdatedConversationStatus` for a conversation: if `new_status` is `InProgress`
     or `TransientError` and no guard is held for that conversation, acquire one with
     reason `"Agent Mode run in-progress"` and store it. If `new_status` is
     `Success`/`Error`/`Cancelled`/`WaitingForEvents`/`Blocked`, drop the guard for that
     conversation (if held). Re-acquire on a later return to `InProgress`. Only acquire
     when no guard exists for the id (never double-acquire for one conversation).
   - On `ClearedConversationsForTerminalSurface` / `RemoveConversation` /
     `DeletedConversation`: drop guards for the affected conversation id(s) so pane
     close / deletion never leaks an assertion.
   - On `ConversationStatusUpdate::Restored` with an active status (`InProgress`/
     `TransientError`): conservatively acquire (the run may resume on app restart). If a
     restored `InProgress` conversation does not actually resume in practice, the guard
     is still bounded — it is released when the conversation transitions or is cleaned up
     (see Open questions resolved).
   - The model's own `Drop` releases all held guards as a final safety net.

2. **Wire the `prevent_sleep` dependency into the `app` crate.**
   - Add `prevent_sleep.workspace = true` to `app/Cargo.toml`. No change to the
     `prevent_sleep` crate itself is required — `Guard` is already constructible via
     `prevent_sleep::prevent_sleep(reason)` and is `Send`/`Sync` (mac) / sendable (win),
     so it can be stored in a model. On wasm/Linux the guard is a no-op, so the model is
     harmless there.

3. **Leave the existing per-request guard in place.**
   - Do **not** remove `.prevent_sleep("Agent Mode request in-progress")` in
     `crates/warp_multi_agent_client/src/lib.rs:59` or the `prevent_sleep::Stream::wrap`
     in `crates/http_client/src/lib.rs:553-557`. The conversation-level guard is the new
     outer scope; the per-request guard stacks cleanly beneath it (on Windows the
     `State` thread aggregates tasks by id, `windows.rs:99-118`; on macOS multiple
     `NSProcessInfo` activities stack). Removing the per-request guard is a non-goal
     follow-up.

**Design alternatives:**

- **Where to attach the conversation-level guard** — (a) *selected:* a new dedicated
  model subscribed to `BlocklistAIHistoryModel` status + cleanup events, keyed by
  conversation id. Centralizes the lifecycle in one testable owner and reuses the
  existing status event surface. (b) Acquire/release the guard directly at every
  status-transition site in `BlocklistAIController`. More invasive, scatters the logic
  across many call sites, and is error-prone (easy to miss a transition). (c) Acquire
  the guard inside `ShellCommandExecutor` per LRC action. Narrower, but it does not cover
  the gap between `StreamFinished` and the first LRC action, nor the gap between LRC
  completion and the next follow-up turn, and requires per-action acquire/release that
  still leaves sub-turn gaps — it does not fully close the gap the requester named. (d)
  Extend `ResponseStream` to hold a conversation-level guard. `ResponseStream` is
  per-turn/per-SSE-stream, so it is the wrong scope (it drops at `StreamFinished` — the
  same hole).
- **Release conditions** — (a) *selected:* release on terminal (`Success`/`Error`/
  `Cancelled`) and on idle-waiting-for-user-input (`WaitingForEvents`, `Blocked`), hold
  across `InProgress` and `TransientError`. Matches the requester's "release on terminal
  / cancellation / idle-waiting-for-user-input" direction and avoids holding the
  assertion forever while a conversation is parked waiting for the user. (b) Hold until
  terminal only (keep through `WaitingForEvents`/`Blocked`). Keeps the assertion while
  the user is away from a parked conversation, but drains battery when the agent is
  genuinely idle waiting for input and no command is running — rejected for the LRC
  scope. (c) Release on every `StreamFinished` and re-acquire on the next turn. This is
  the status quo and is exactly the hole being closed.
- **Keep vs remove the per-request guard** — (a) *selected:* keep it (defense in depth,
  lower regression risk, stacks cleanly). (b) Remove it now that the conversation guard
  is the outer scope. Cleaner but widens the blast radius of this change; deferred to a
  follow-up.

**Open questions resolved:**

- *System-only vs system+display sleep* — per the requester's scope, this change
  prevents *system* sleep only. The macOS `IdleDisplaySleepDisabled` flag and the Windows
  `ES_DISPLAY_REQUIRED` flag are explicitly **out of scope** (deferred). The existing
  `NSActivityOptions::UserInitiated` and `ES_SYSTEM_REQUIRED` behavior is unchanged.
- *Aggressive whole-conversation prevention vs minimal gap-covering* — per the requester,
  hold the guard across the conversation/turn lifecycle spanning the LRC gap (the local
  command-execution window between turns), releasing on terminal/cancel/idle-waiting-for-
  user-input. This is the "cover the gap" direction scoped to the conversation lifecycle.
- *Interaction with auto-handoff-on-sleep* — out of scope to redesign; this change adds
  no new handoff behavior or setting. It is complementary: handoff already skips LRCs
  (`auto_handoff.rs:328-330`), so covering the LRC gap here does not change handoff's
  decision. The validation criteria require confirming no regression.
- *Linux keep-awake backend* — out of scope (deferred). Linux stays a no-op; the new
  model compiles and runs on Linux (no-op guard) and the unit tests run on Linux.
- *Remove the existing per-request guard* — no; keep as defense in depth (non-goal
  follow-up).
- *`Blocked` state* — release the guard there (the agent is waiting for the user to
  approve an action; no local command is executing). Re-acquire if the conversation
  returns to `InProgress` after approval.
- *Restored `InProgress` conversations on app restart* — conservatively acquire (the run
  may resume). The implementor should confirm whether restored `InProgress` conversations
  actually resume; if they do not, the guard is still released on transition/cleanup, so
  it is bounded. Recorded as an assumption for the implementor/reviewer to confirm.
- *Reason string* — `"Agent Mode run in-progress"` (distinct from the per-request
  `"Agent Mode request in-progress"`) so OS assertion logs (`pmset -g assertions`,
  `powercfg /requests`) distinguish the two scopes.

**Risks / mitigations:**

- *Guard leak (battery drain)* — if a conversation ends without an observable status
  event, the guard could be held indefinitely. Mitigation: subscribe to all cleanup
  events (`ClearedConversationsForTerminalSurface`, `RemoveConversation`,
  `DeletedConversation`), release on any non-active status, and release all guards in
  the model's `Drop`. The unit test covers the cleanup paths.
- *Double-acquire / panic* — the model only acquires when no guard exists for the
  conversation id. On Windows the `State` thread handles `AddTask`/`RemoveTask` by task
  id (`windows.rs:99-118`); on macOS multiple `NSProcessInfo` activities stack. No panic
  path.
- *Restored stale guard* — a restored `InProgress` conversation that never resumes would
  hold a guard until cleanup. Bounded by cleanup/transition; the implementor confirms the
  resume behavior.
- *Wasm / Linux* — guard is a no-op there; ensure the model compiles on wasm and that no
  native-only API is unconditionally called (gate any native-only behavior behind
  `cfg(not(target_family = "wasm"))` if needed, though the no-op `Guard` makes this
  likely unnecessary).
- *Battery impact of holding across the whole conversation* — intended behavior per the
  requester (close the gap). Mitigated by releasing on `WaitingForEvents`/`Blocked`, so a
  conversation parked waiting for the user does not hold the assertion.

## Validation & verification criteria (all must pass before merge)

1. *Reproduction fixed (manual, macOS)* — ask the agent to run a long-running local
   command (e.g. `sleep 600`). During the command run — specifically in the inter-turn
   gap after the `RunShellCommand` turn's SSE stream finishes and before the follow-up
   turn — run `pmset -g assertions` in another terminal and confirm a
   `PreventUserIdleSystemSleep` assertion from Warp is present **continuously** through
   the `sleep 600` run (not only while an SSE turn is streaming). This carries forward
   the triage's manual corroboration as the repro. (Behavioral proof is the OS power
   assertion output, not a screenshot — this change has no visible UI delta.)
2. *Reproduction fixed (manual, Windows)* — equivalent on Windows: `powercfg /requests`
   shows a `SYSTEM` request from Warp continuously through a long-running command run,
   including the inter-turn gap.
3. *Regression test (unit) — guard lifecycle* — add a unit test (e.g.
   `agent_run_sleep_guard_model_lifecycle` in the new model's `*_tests.rs`) that drives
   `BlocklistAIHistoryModel` conversation status transitions and asserts the guard map:
   acquires on `InProgress`; holds across `TransientError`; drops on
   `Success`/`Error`/`Cancelled`/`WaitingForEvents`/`Blocked`; re-acquires on a
   subsequent return to `InProgress`. The test must fail before the change (no model →
   no guard held during the `InProgress` local-execution window) and pass after. The
   implementation may add a test seam (e.g. a guard acquire/release counter or a
   mockable backend) to make the presence/absence assertion concrete on the Linux/no-op
   target.
4. *Regression test (unit) — cancel releases* — a unit test asserting that the
   cancellation path (`cancel_conversation_progress` → `Cancelled`) on an `InProgress`
   conversation drops the guard for that conversation.
5. *No leak on cleanup (unit)* — a unit test asserting guards are dropped on
   `ClearedConversationsForTerminalSurface` (for each `cleared_conversation_id`),
   `RemoveConversation`, and `DeletedConversation`, so closing the pane / deleting the
   conversation does not hold a wake assertion.
6. *Per-request guard unchanged (no collateral damage)* — the existing per-request guard
   in `crates/warp_multi_agent_client/src/lib.rs:59` and
   `crates/http_client/src/lib.rs:553-557` is **not** removed. Checked by: the existing
   `http_client` and `warp_multi_agent_client` tests still pass
   (`cargo nextest run -p http_client -p warp_multi_agent_client`); reasoning that the
   conversation-level guard stacks cleanly with the per-request guard (Windows `State`
   aggregates tasks by id; macOS activities stack) — no double-acquire, no leak.
7. *Auto-handoff-on-sleep not regressed* — the LRC skip at
   `app/src/workspace/auto_handoff.rs:328-330` is unchanged and no new setting/flag
   affecting handoff gating is introduced. Checked by: the existing `auto_handoff` tests
   pass; reasoning that the change is complementary (handoff already skips LRCs, so
   covering the LRC gap here does not alter handoff's decision).
8. *Wasm no-op / compiles* — the new model compiles and registers on wasm without
   changing behavior (the guard is a no-op on wasm). Checked by: a wasm build of the
   `app` crate (the repo's documented wasm target/check) succeeds.
9. *Repository checks (scope-proportional per the factory-verification mandate)* —
   `./script/format` (or `cargo fmt --all --check`) passes; `cargo clippy --workspace
   --all-targets --all-features --tests -- -D warnings` passes on the touched crates; the
   focused nextest suite for the touched modules (the new guard model, the
   `app/src/ai/blocklist` controller/history_model wiring, `prevent_sleep`) passes. The
   repo's `./script/presubmit` / PR CI is the full-suite backstop.
