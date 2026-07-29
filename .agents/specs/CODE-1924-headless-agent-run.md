*Spec: Headless non-interactive agent run in Warp Agent CLI*

== PRODUCT ==

*Summary:* Add a one-shot `warp run` subcommand to the standalone Warp Agent CLI/TUI binary. It accepts an instruction from a positional argument, piped stdin, or both; runs Warp's built-in Oz agent without mounting the interactive TUI; and provides script-safe text or JSON Lines output. The implementation must reuse the app crate's in-process agent runner while preserving the TUI's authentication, settings, execution-profile, persistence, AgentSource, and indexing behavior.

*Key design choices:*
- `warp run [PROMPT]` is a new subcommand of the standalone TUI binary, not an alias for the separate Oz CLI's `warp agent run` command and not a subprocess invocation of another Warp binary.
- Unattended actions use the selected TUI execution profile and organization policy. Anything that still requires human approval is blocked rather than prompting, falling back to a TUI, or silently escalating.
- Default output follows the script-friendly Codex contract: progress and diagnostics go to stderr while stdout contains only the final assistant response. `--output-format jsonl` exposes the existing agent event stream as a machine-readable protocol with a terminal event.

*Behavior*:
1. The installed standalone Warp Agent CLI accepts:
   - `warp run [--profile <ID>] [--output-format <text|jsonl>] [PROMPT]`
   - The existing global `--api-key <KEY>` / `WARP_API_KEY` authentication option remains available before or after `run`.
   - `text` is the default output format. No shorthand or compatibility form for `warp agent run` is added.
2. Prompt input is resolved before app initialization:
   - With a non-empty positional `PROMPT` and terminal stdin, the positional value is the prompt.
   - With no positional value and piped stdin, the complete UTF-8 stdin value is the prompt.
   - With both a positional value and piped stdin, the positional value is the instruction and stdin is additional context. The model-visible string is the positional value, two newline characters, then stdin, preserving each non-empty source's contents.
   - An absent or whitespace-only effective prompt, unreadable stdin, or non-UTF-8 stdin is a usage/input error. It must not start an agent or mount the TUI.
   - Piped stdin is limited to 10 MiB. Exceeding the limit fails clearly and nonzero instead of buffering unbounded input.
3. `run` is strictly headless and one-shot. It initializes the shared app model graph, starts one local Oz conversation in the process's current working directory, waits for its terminal status, and exits. It never enters alternate-screen mode, reads interactive keystrokes, prints a resume command, or mounts `RootTuiView`.
4. Authentication remains TUI-scoped:
   - The run uses `--api-key`, `WARP_API_KEY`, or already-persisted Warp Agent CLI credentials from the TUI secure-storage namespace.
   - Missing, expired, or invalid credentials fail on stderr and with a nonzero status. The headless command never starts device login, opens a browser, or waits for interactive authentication.
5. Agent configuration remains TUI-scoped:
   - With no `--profile`, the TUI default execution profile supplies the model and permissions.
   - `--profile <ID>` selects an existing file-backed TUI execution profile by stable ID. An unknown ID is an error; it must not fall back to another profile.
   - Organization autonomy overrides, protected-path rules, allowlists, and denylists continue to take precedence exactly as they do in the interactive TUI.
   - v1 does not expose separate model, MCP, skill, environment, harness, conversation, cwd, share, or auto-approval flags from the Oz CLI.
6. Permission handling is fail-closed without an interactive approver:
   - Actions the effective execution profile can autoexecute continue normally.
   - An action classified as requiring approval, an explicit denial, or an `AskUserQuestion` request transitions the run to a blocked terminal result with a useful action/reason description.
   - A blocked run does not display a permission prompt, wait on stdin, launch the TUI, or reinterpret the policy as approval.
   - Users who intentionally need broader autonomy configure and explicitly select a TUI execution profile that grants it. There is no blanket “dangerously skip permissions” flag in v1.
7. Default `text` output is pipeline-safe:
   - stdout contains only the final assistant-authored text, in message order, followed by one newline. It excludes reasoning, conversation/run IDs, tool calls and results, TODOs, progress, diagnostics, and log paths.
   - Human-readable progress and tool activity may stream to stderr. Errors and blocked reasons are reported on stderr.
   - A successful response with no assistant text produces empty stdout rather than substituting progress or metadata.
8. `jsonl` output is a streaming protocol:
   - Every stdout line is one complete JSON object. Existing stable agent/tool/system event representations are reused rather than exposing internal Rust types.
   - stderr is reserved for diagnostics that are outside the protocol; no human-readable progress is mixed into stdout.
   - Once app initialization begins, stdout ends with exactly one system terminal event: `run_completed`, `run_blocked`, `run_cancelled`, or `run_failed`. A successful terminal event contains the complete final assistant text; blocked/failed events contain a stable reason string. Pre-initialization argument, stdin, or authentication-bootstrap failures may produce no JSONL records and are reported on stderr.
9. Exit status is zero only when the one-shot agent conversation reaches successful completion. Argument/input failures, authentication failures, unknown profiles, permission-blocked runs, cancellations, agent errors, and app/driver failures exit nonzero. Signal termination retains the platform's normal signal-derived exit behavior; the command must never convert interruption into success.
10. The headless command preserves the existing TUI launch identity:
    - It continues to use `LaunchMode::Tui`, `ExecutionMode::Tui`, TUI settings, TUI persistence, TUI execution profiles, the `.tui` secure-storage service suffix, and the TUI logging frontend.
    - It keeps the TUI's current `AgentSource` value (`None`); it does not claim the Oz CLI's `Cli` source or introduce a headless-TUI source in this change.
    - Codebase indexing remains disabled for the TUI because persisted-index restore and snapshot writes are not safe across concurrent GUI/TUI processes. Project/global rules and skills discovery continue through the non-index-backed context paths already initialized for the TUI.
11. Existing modes remain compatible. A bare `warp`, `warp --resume <TOKEN>`, provider API-key maintenance flags, `--help`, `--version`, and worker re-exec dispatch retain their current behavior. `run` conflicts structurally with interactive/resume and provider-key maintenance modes rather than attempting to compose with them.
12. v1 deliberately excludes session resume or follow-up turns, detached/background execution, interactive fallback, cloud runs, third-party harnesses, full Oz CLI flag parity, a GUI monitor, codebase indexing, and bundling/shelling out to the Oz CLI.

== TECH ==

*Context:* Research is pinned to `warpdotdev/warp` commit `08352769463a80488d30d2f720b914613431b643`.
- `crates/warp_tui/src/session.rs:39-190 @ 08352769463a80488d30d2f720b914613431b643` defines the current flat Clap surface, dispatches worker invocations before normal argument parsing, routes provider-key maintenance through `warp::run_tui_cli_command`, and otherwise always calls `warp::run_tui` to mount the interactive frontend.
- `crates/warp_tui/src/session_tests.rs:1-112 @ 08352769463a80488d30d2f720b914613431b643` covers the existing parsing, conflicts, resume token, help, and version contracts.
- `crates/warp_tui/Cargo.toml:10-45 @ 08352769463a80488d30d2f720b914613431b643` shows that all channel-specific TUI binaries share the `warp_tui::run` entrypoint and that `warp_tui` already links the `warp` app library with its `tui` feature. No executable-to-executable dependency is needed.
- `app/src/lib.rs:339-428 @ 08352769463a80488d30d2f720b914613431b643` maps every TUI launch to no `AgentSource`, defines `LaunchMode::Tui` / `TuiEntryPoint`, and carries an API key only for the interactive entrypoint today.
- `app/src/lib.rs:566-591 @ 08352769463a80488d30d2f720b914613431b643` intentionally disables indexing for all TUI launches because persisted index state is not multi-process safe.
- `app/src/lib.rs:902-928,1306-1369,1428-1476 @ 08352769463a80488d30d2f720b914613431b643` provides the shared TUI initializer and command callback, then selects the API key, AgentSource, TUI settings, `.tui` secure-storage namespace, and TUI persistence scope from the launch mode.
- `app/src/lib.rs:2134-2144,2381-2432 @ 08352769463a80488d30d2f720b914613431b643` disables `RepoOutlines` indexing and persisted index restoration while still initializing project-context and persisted-workspace models.
- `app/src/ai/execution_profiles/mod.rs:94-145 @ 08352769463a80488d30d2f720b914613431b643` seeds the fresh TUI default profile, including `AgentDecides` command execution plus the user's local allowlists/denylists.
- `app/src/ai/execution_profiles/profiles.rs:67-111,247-279 @ 08352769463a80488d30d2f720b914613431b643` makes file-backed settings profiles authoritative for every TUI launch.
- `app/src/ai/blocklist/permissions.rs:20-127,174-221 @ 08352769463a80488d30d2f720b914613431b643` centralizes command/file permission reasons and overlays organization policy on execution profiles.
- `app/src/ai/agent_sdk/mod.rs:130-141,292-340,1520-1684 @ 08352769463a80488d30d2f720b914613431b643` is the canonical in-process agent command path: it checks auth, refreshes account state, creates `AgentDriverRunner`, starts the driver, reports fatal failures, and terminates the app.
- `app/src/ai/agent_sdk/driver.rs:3278-3284,3463-3567 @ 08352769463a80488d30d2f720b914613431b643` currently sends conversation metadata, inputs, and completed exchange outputs to stdout for every output format.
- `app/src/ai/agent_sdk/driver/output.rs:470-899 @ 08352769463a80488d30d2f720b914613431b643` contains the human-readable formatters and the intentionally stable JSON event representation that should back the new output router.
- `crates/warp_cli/src/agent.rs:20-74,331-458 @ 08352769463a80488d30d2f720b914613431b643` defines the broader Oz CLI output and run arguments. The TUI command should reuse its runner semantics in-process without exposing this much larger CLI surface.

Primary-source precedent also informs the contract:
- Claude Code non-interactive mode accepts a positional instruction plus piped context, defaults to final plain text, offers structured and streaming JSON formats, and uses a conservative `dontAsk` mode for preapproved-only automation: https://code.claude.com/docs/en/headless and https://code.claude.com/docs/en/permission-modes
- Codex `exec` treats a positional value as the instruction and piped stdin as additional context, streams progress to stderr, reserves stdout for the final response, offers JSONL events, and requires broader permissions to be selected explicitly: https://developers.openai.com/codex/noninteractive and https://developers.openai.com/codex/agent-approvals-security

*Design alternatives*:
- *Binary boundary:* Invoking or bundling the Oz CLI would reuse its command parser but violates the standalone TUI binary boundary, complicates distribution, and forks TUI settings/auth identity. Reimplementing an agent loop in `warp_tui` would duplicate critical lifecycle logic. Selected: add a narrow TUI-facing app API that constructs the existing local Oz run in-process and delegates to `agent_sdk` / `AgentDriver`.
- *Command shape:* A root `--headless`/`--print` flag is close to Claude Code but composes ambiguously with `--resume` and maintenance flags. `warp agent run` mirrors the separate Oz CLI but exposes the wrong product hierarchy and invites unsupported flag parity. Selected: the explicit `warp run` subcommand requested by the product owner.
- *Permission policy:* A fixed read-only policy is safe but ignores users' TUI configuration; automatically approving the TUI default is unsafe; a new headless policy store would drift from interactive behavior. Selected: resolve the effective TUI profile (plus organization overrides), run only already-authorized actions, and convert every remaining approval request to a blocked result. An optional `--profile` makes elevated autonomy explicit without adding a bypass.
- *Output contract:* Reusing the current SDK stdout stream unchanged makes shell pipelines consume progress and tool results rather than the answer. A single final JSON document is easy to parse but provides no progress for long runs. Selected: final-only stdout plus stderr progress by default, with an opt-in JSONL event stream. Expose `--output-format` rather than a `--json` boolean so the values are self-describing and extensible.
- *Prompt and stdin:* Requiring exactly one source is simple but prevents the established “instruction plus piped context” workflow. Treating concatenated bytes as one opaque prompt makes ordering unclear. Selected: positional instruction first, then two newlines, then piped context, matching documented Codex semantics and making tests deterministic.
- *Launch identity and indexing:* Reclassifying the run as `LaunchMode::CommandLine` would turn on Oz CLI `AgentSource::Cli`, GUI settings/persistence, and indexing. A new launch/source variant creates telemetry and storage semantics not requested in v1. Selected: keep `LaunchMode::Tui` and its current `AgentSource::None`; extend only its command entrypoint to carry authentication and a typed one-shot callback. Keep indexing off for the already-documented concurrency reason.

*Proposed changes:*
1. In `crates/warp_tui/src/session.rs`, replace the flat mutually-exclusive launch fields with an optional Clap subcommand while preserving global auth and existing maintenance behavior. Add a small `RunArgs` containing positional `prompt`, `--profile`, and `--output-format`; keep worker dispatch before parsing. Resolve bounded stdin through a pure helper so source ordering, empty input, UTF-8 errors, and the 10 MiB limit are unit-testable without touching process-global stdin.
2. Add a narrow, typed TUI agent-run request to `app/src/tui_export.rs` (prompt, optional profile ID, output mode) and an app-level `run_tui_agent` wrapper alongside `run_tui` / `run_tui_cli_command`. The wrapper creates `LaunchMode::Tui`, carries the optional API key into app initialization, and dispatches only the local Oz runner after initialization. It must not expose or clone the full `RunAgentArgs` surface into the TUI crate.
3. Extend `TuiEntryPoint::CliCommand` (or replace it with equally narrow typed command variants) so command-style TUI launches can supply `api_key`. Update `api_key_from_launch_mode` and launch-mode unit tests while leaving settings mode, storage namespace, persistence scope, execution mode, AgentSource, and `supports_indexing()` unchanged.
4. Factor the minimum reusable local-run construction out of `agent_sdk` if the TUI wrapper cannot safely build `CliCommand::Agent(AgentCommand::Run)` without fabricating unrelated Oz flags. The shared path must retain authentication refresh, task/driver setup, profile selection, terminal lifecycle, error propagation, and app termination. No agent loop or permission decision is implemented in `warp_tui`.
5. Add an output policy/sink at the `AgentDriver` boundary rather than redirecting process-wide stdout. For text mode, route non-final formatted events to stderr and accumulate only assistant `Text` messages for the final stdout write. For JSONL, continue using the stable JSON formatter on stdout and add one terminal system-event representation emitted from the single completion/error path. Ensure buffered writes flush before termination and write failures propagate as run failures.
6. At the permission-prompt boundary, detect the non-interactive TUI command and resolve prompt-requiring actions as `Blocked` using the existing action description/reason. Preserve the normal interactive TUI executors for bare `warp`; do not globally change execution-profile semantics. Treat `AskUserQuestion` the same way so no action can wait indefinitely for absent input.
7. Add a headless-run startup telemetry discriminator if needed for product measurement, but retain the TUI frontend/execution mode and current `AgentSource`. Never emit prompt, stdin, API-key, or tool payload contents in startup telemetry.

*Open questions resolved:*
- The product owner chose `warp run <prompt>` plus stdin and required an in-process shared-library implementation in the separate TUI binary; bundling or invoking the Oz CLI is forbidden.
- The product owner required the existing TUI AgentSource and normal TUI indexing behavior. Code research resolves those values to `AgentSource::None` and indexing disabled; project/global rules and skills remain available without codebase indexing.
- The permission contract follows the conservative common denominator of Claude Code's preapproved-only `dontAsk` behavior and Codex's explicit sandbox/profile selection: use existing TUI profiles, never infer approval, and block when interaction would be required.
- The output contract follows Codex's clean stdout/stderr split for the default and both products' JSONL streaming precedent. A separate final JSON object mode is not included in v1.
- v1 is one-shot only. Resume, follow-up input, detach/background execution, and interactive fallback are explicitly out of scope.
- No further implementation constraint was requested beyond preserving the standalone TUI binary boundary.

*Risks / blast radius:*
- Refactoring driver output can change the established Oz CLI protocol. Mitigation: make the sink/output policy explicit per caller, preserve the current Oz CLI policy unchanged, and add byte-for-byte tests for both existing and TUI-specific routes.
- Permission prompts can occur through commands, file edits/reads, MCP tools, terminal writes, computer use, and `AskUserQuestion`. Missing one can hang CI. Mitigation: centralize “interactive approver available” in the execution context and test representative prompt-producing action families plus an exhaustive match over permission-bearing action types.
- Initialization currently treats all TUI logs as file-only because interactive rendering owns stdout/stderr. Headless output needs stderr without allowing ordinary logs to corrupt it. Mitigation: keep logs file-backed and write only intentional progress/diagnostics through the output sink.
- Extending `TuiEntryPoint` affects provider-key maintenance and interactive auth. Mitigation: preserve typed entrypoints, add launch-mode tests for API-key extraction and storage identity, and keep all existing TUI session tests.
- Input buffering can consume excessive memory or block on a terminal. Mitigation: read only non-terminal stdin, enforce the 10 MiB cap while reading, and fail before full app bootstrap.
- The current `AgentSource::None` is an acknowledged TUI gap. This change deliberately preserves it; source taxonomy can be addressed separately without coupling it to headless execution.

*Validation & verification criteria* (must ALL pass before merge):
1. Parser tests in `crates/warp_tui/src/session_tests.rs` prove `warp run "answer this"`, `--profile`, both output values, and global `--api-key` placement parse as specified; `warp agent run`, unknown output values, and composition with resume/provider-key maintenance are rejected. Existing help, version, resume, bare startup, and provider-key tests remain green. This verifies behavior #1 and #11.
2. Pure prompt-resolution tests cover positional-only, stdin-only, positional-plus-stdin with the exact two-newline delimiter, empty/whitespace-only sources, an empty pipe, invalid UTF-8, a read error, exactly 10 MiB, and 10 MiB plus one byte. The oversize/error cases prove no app callback is invoked. This verifies behavior #2.
3. A process-level TUI binary test proves worker argv dispatch still wins before subcommand parsing and a prompt matching a worker name later in argv does not trigger worker mode. Existing process-level worker coverage remains green. This verifies behavior #11.
4. Launch-mode tests in `app/src/lib_tests.rs` cover interactive TUI, provider-key command, and headless agent command API-key extraction; all three retain `SettingsMode::Tui`, `.tui` secure storage, TUI logging/execution mode, `AgentSource::None`, and `supports_indexing() == false`. This verifies behavior #4 and #10.
5. App/agent SDK tests with mocked auth prove persisted TUI auth and explicit API-key auth can start a run; missing/invalid auth terminates nonzero without mounting a view or initiating interactive login. This verifies behavior #3 and #4.
6. Execution-profile tests prove omitted `--profile` selects the TUI default, a valid stable ID selects that file-backed TUI profile, an unknown ID fails without fallback, and organization overrides/denylists still win. This verifies behavior #5.
7. Non-interactive permission tests exercise at least an allowed read/command, a profile-allowed edit, an always-ask command or edit, an explicit denylist match, a protected-path write, and `AskUserQuestion`. Allowed actions proceed; every approval/denial case reaches `Blocked`, emits a reason, exits nonzero, and never creates a permission UI or reads stdin. An exhaustive match/test ensures every permission-bearing action family has a headless disposition. This verifies behavior #6 and #9.
8. Output-router unit tests feed synthetic conversation metadata, reasoning, tool calls/results, TODO/progress, multiple assistant text segments, empty final text, success, blocked, cancelled, and error statuses:
   - text mode sends only concatenated final assistant text plus one newline to stdout, sends permitted progress/errors to stderr, and leaves stdout empty when no final text exists;
   - JSONL mode parses every stdout line independently as JSON, preserves existing event objects, emits no human text to stdout, and ends with exactly one correctly typed terminal system event containing final text or reason.
   Existing Oz CLI formatter/output tests must prove its current streaming stdout behavior is unchanged. This verifies behavior #7 and #8.
9. Completion-path tests prove success is the only zero-status outcome; blocked, cancelled, auth, profile, driver, output-write, and agent failures propagate through app termination as errors. A subprocess smoke test confirms no alternate-screen escape sequence, resume hint, or interactive prompt is emitted by `warp run`. This verifies behavior #3, #6, #8, and #9.
10. Context initialization tests prove a headless TUI run does not register active codebase indexing or restore persisted indices, while project/global rules and skills discovery still initialize as they do for interactive TUI. This verifies behavior #10.
11. The implementation contains no process spawn of an Oz/Warp CLI binary and introduces no dependency from `warp` back to `warp_tui`. Dependency inspection (`cargo tree -p warp_tui`) shows the existing one-way `warp_tui -> warp` library boundary. This verifies behavior #1, #3, and #12.
12. Headless/manual smoke verification with a test account exercises positional-only, stdin-only, combined input, text redirection, JSONL parsing, a permissive profile, and a blocked default-profile action. Captured stdout/stderr and statuses match behaviors #1-#9. This is a terminal/headless feature, so GUI `computer_use` and screenshots are not required.
13. Repository gates pass from the workspace root: `./script/format`, the exact workspace Clippy command used by `./script/presubmit`, `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`, and `cargo test --doc`. The PR's full CI is the final cross-platform backstop.
