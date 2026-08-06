*Spec: warp client skips IAP when pointed at a local (non-IAP) server (REMOTE-2540)*

This is the **warpdotdev/warp (client)** half of REMOTE-2540. It defines the
client-side contract that lets a Dev/staging build talk to a local `oz-local`
server without attempting an IAP connection. The sibling **warpdotdev/warp-server**
spec (oz-dev PATH resolution + docs) is written against the contract defined
here. Linear: https://linear.app/warpdotdev/issue/REMOTE-2540

== PRODUCT ==
*Summary:* A Dev-channel (`oz-dev`) build compiles in an `IapConfig`, so today the
client unconditionally builds `IapState` and, for interactive clients, gates
startup user authentication on IAP access — **independent of which server it is
pointed at**. When `warp-server`'s `./script/oz-local` launches such a build
against the local `oz-local` server (`http://localhost:8080`, not behind IAP,
no IAP credentials available), the IAP token mint/refresh fails and the client
errors out ("IAP credential refresh failed") instead of talking to the local
server. This change makes the client **auto-disable IAP whenever the resolved
server root URL is a local, non-IAP host**, so pointing the build at
`localhost` transparently skips IAP with no new flag or env var. IAP stays fully
enforced for every non-local server URL, including the real staging host.

*Key design choices:*
1. **Auto-disable on local server URL — the resolved server URL is the sole
   authority (requester chose option A).** IAP is enabled only when the build has
   an `IapConfig` **and** the resolved `server_root_url` host is not local. There
   is **no** opt-out env var or flag anywhere — the requester explicitly rejected
   an explicit opt-out. This makes it structurally impossible to disable IAP
   against staging: the only way to skip IAP is to point the server URL at a
   local host, which by definition is not staging.
2. **Single gate point.** The entire contract collapses to one line —
   `initialize_app` builds `iap_state` (`warp/app/src/lib.rs:1483 @ 5f873432950627fcf5405ccf5d38432b7ae386b7`).
   Gating that one `Option` to `None` for a local server URL disables IAP end to
   end: `IapManager::is_enabled()` becomes `false`, the startup auth gate runs
   normal user auth immediately, and even the runner WIF self-mint path goes
   inert (see Tech). No other call site changes.
3. **Conservative local-host allowlist (default IAP-on).** "Local" is a tight
   explicit allowlist — `localhost`, `127.0.0.1`, `::1`, `host.docker.internal`.
   Every other host — including any staging/prod host and anything unrecognized —
   keeps IAP. Unknown hosts fail safe toward IAP-enabled, so a new server host
   can never silently lose IAP.

*Behavior* (numbered, testable invariants from the consumer's view):
1. On a build that has an `IapConfig` (Dev/staging), when the resolved
   `server_root_url` host is `localhost`, `127.0.0.1`, `::1`, or
   `host.docker.internal`, the client does **not** create `IapState`: no IAP token
   mint/refresh is attempted and no "IAP credential refresh failed" toast appears.
2. In that local-server case, interactive startup user authentication proceeds
   **immediately** against the local server (the normal user-auth flow runs; IAP
   is not a pre-gate). Authentication is not bypassed — only the IAP gate is removed.
3. On the same build, when the resolved `server_root_url` host is the real
   staging host (`staging.warp.dev`) — or any other non-local host — IAP is
   created and enforced exactly as it is today (no behavior change).
4. There is no env var or flag that can enable or disable IAP. The decision is a
   pure function of `(iap_config present, resolved server_root_url)`. A stale
   environment cannot disable IAP against a non-local server.
5. Builds with no `IapConfig` (production/release channels: Stable, Preview, Oss)
   are unaffected — they never had IAP and never reach the new local-host check.
6. Release channels (Stable/Preview/Oss) additionally cannot even reach a local
   server URL, because they ignore server-URL overrides
   (`Channel::allows_server_url_overrides()` is `false` for them) — so the
   auto-disable is unreachable on release builds (defense in depth).
7. In a sandboxed Oz runner (`OZ_RUN_ID` set) pointed at a local server, the
   runner Workload-Identity-Federation IAP mint is inert (no self-mint occurs),
   because it is only ever driven through the same disabled `IapManager`.

== TECH ==
*Context:* All references pinned to warp commit
`5f873432950627fcf5405ccf5d38432b7ae386b7` (branch `master`).

How the IAP decision flows today (end-to-end trace, verified against current code):
- **Server-URL override is applied before app init.** `run()` parses args, then
  — only when `Channel::allows_server_url_overrides()` is true (Dev/Local/
  Integration) — applies `--server-root-url` / `WARP_SERVER_ROOT_URL` via
  `ChannelState::override_server_root_url(...)`
  (`warp/app/src/lib.rs:762-767 @ 5f87343`). Both the flag and the env feed the
  same clap arg (`env = "WARP_SERVER_ROOT_URL"`,
  `warp/crates/warp_cli/src/lib.rs:142-148 @ 5f87343`). This runs **before**
  `run_internal(...)` → `initialize_app(...)` (`lib.rs:802/834` → `:1410`), so by
  the time the IAP gate runs the resolved `server_root_url` already reflects the
  local override. **This is the critical ordering finding: the client does see
  the local URL at the point the IAP gate runs**, so a URL-keyed gate is viable
  with no restructuring. `oz-local` exports
  `OZ_LOCAL_SERVER_ROOT_URL=http://localhost:8080` and the worker passes it as
  `WARP_SERVER_ROOT_URL` (warp-server side), so the override lands.
- **IAP is built purely from the compiled-in config, ignoring the URL.**
  `initialize_app` builds `iap_state` as
  `ChannelState::iap_config().map(|cfg| Arc::new(IapState::new(&cfg)))`
  (`lib.rs:1482-1485 @ 5f87343`). `iap_config()` returns the build's compiled-in
  `Some(IapConfig)` for staging builds regardless of server URL
  (`warp/crates/warp_core/src/channel/state.rs:226-228 @ 5f87343`;
  `IapConfig` "present only on staging builds",
  `warp/crates/warp_core/src/channel/config.rs:30-37,50-53 @ 5f87343`).
- **`iap_state` propagates to `ServerApiProvider` and `IapManager`**
  (`lib.rs:1487-1491`, `:2338-2352`). The runner WIF mint `managed_iap_mint`
  (`lib.rs:2321-2327`, gated on a valid `OZ_RUN_ID`) is only ever consumed inside
  `IapManager` (`:2351`).
- **Interactive startup auth is gated on IAP.**
  `authenticate_user_after_iap_access` (`lib.rs:1378-1407`) calls
  `authentication.start(ctx)` immediately when
  `!iap_manager.is_enabled() || iap_manager.has_valid_token()`, otherwise it
  subscribes and calls `iap_manager.ensure_access(ctx)` (`:1406`). Against the
  local server, `ensure_access` triggers a mint/refresh that fails, producing the
  reported error via the `RefreshFailed` toast (`lib.rs:2354-2375`).
- **Why gating one Option is sufficient.**
  `IapManager::is_enabled()` is `self.state.is_some()`
  (`warp/crates/warp_server_client/src/iap.rs:233-235 @ 5f87343`). If `iap_state`
  is `None`: (a) `is_enabled()` is `false`, so `authenticate_user_after_iap_access`
  runs normal auth immediately (invariant #2); (b) `IapManager::start_refresh`
  early-returns on `let Some(state) = self.state ... else { return }`
  (`iap.rs:285-288`), so even with `managed_iap_mint` present the WIF self-mint
  never runs (invariant #7); (c) `ServerApi` gets `None` IAP state, so no
  proxy-auth header is attached. Setting `iap_state = None` for a local URL is the
  whole fix.

*Design alternatives* (per decision point with more than one reasonable approach):
- **Opt-out mechanism — A (auto-disable on local URL) vs B (explicit env/flag) vs
  C (both).**
  - *A — auto-disable on resolved local server URL (CHOSEN; requester picked A,
    verbatim "I like A, let's auto disable when resolved URL is local").* No new
    surface; `./script/oz-local` "just works". The URL is the sole authority, so
    IAP can never be disabled against staging. Smallest, safest change.
  - *B — explicit `WARP_DISABLE_IAP` env / `--no-iap` flag (REJECTED).* Most
    dangerous: a stale `WARP_DISABLE_IAP` left in a shell could silently disable
    IAP against real staging. Adds a new surface warp-server would have to set.
  - *C — both, auto-detect with an explicit override (REJECTED for v1).* An
    override that can force-disable reintroduces B's staging risk; an override
    that only narrows (never force-enables against local) adds surface with no
    benefit for the requester's use case. If a future need appears, an override
    that is a **no-op against non-local URLs** could be added without breaking
    this contract.
- **What counts as "local" — tight host allowlist (skip-IAP set, default IAP-on)
  vs staging-only allowlist (IAP-on set, default IAP-off).**
  - *Skip-IAP host allowlist — `{localhost, 127.0.0.1, ::1, host.docker.internal}`,
    everything else keeps IAP (CHOSEN).* Fails safe toward IAP-enabled: a new or
    unrecognized host keeps IAP, so staging can never silently lose it. Directly
    matches the requester's "auto-disable when the URL is local".
  - *Enable-IAP only when host == `staging.warp.dev` (REJECTED).* `uses_staging_server()`
    already exists (`state.rs:119-124`) but inverting the default to IAP-off means
    any new IAP-gated host (a second staging domain, an ngrok tunnel to staging)
    would silently run without IAP — the opposite of the safety requirement.
- **Where the decision lives / helper granularity.**
  - *Pure free function classifier + thin `ChannelState` wrapper (CHOSEN).* Add a
    pure `host_is_local(server_root_url: &str) -> bool` free function in
    `state.rs` (mirroring the existing `derive_http_origin_from_ws_url` free fn
    tested in `state_tests.rs`), plus `ChannelState::server_root_url_is_local()`
    that applies it to `Self::server_root_url()`. The `initialize_app` gate calls
    the wrapper. Keeps the branching logic fully unit-testable without touching
    global `CHANNEL_STATE`.
  - *Inline the host check at `lib.rs:1483` (REJECTED).* Not unit-testable
    (`initialize_app` is heavy app bootstrap); duplicates URL parsing.

*Proposed changes:*
1. **Local-host classifier — `warp/crates/warp_core/src/channel/state.rs`.** Add a
   pure free function `fn host_is_local(server_root_url: &str) -> bool` that parses
   the URL and returns `true` iff `Url::host_str()` matches one of the allowlisted
   local hosts: `localhost`, `127.0.0.1`, `::1` (note: `url` yields the IPv6 host
   as `[::1]` — the implementation must match whatever `host_str()` returns and
   the unit test pins it), and `host.docker.internal`. Unparseable input returns
   `false` (fail safe → IAP stays on). Keep it a free fn so `state_tests.rs`
   (which is `#[cfg(all(test, not(feature = "test-util")))]`) can test it directly
   without the `test-util` mock-server routing that `server_root_url()` uses.
2. **`ChannelState` accessor — same file.** Add
   `pub fn server_root_url_is_local() -> bool { host_is_local(&Self::server_root_url()) }`
   next to `uses_staging_server()`.
3. **Gate the IAP decision — `warp/app/src/lib.rs`.** Change `iap_state`
   construction (`:1482-1485`) so `IapState` is created only when an `IapConfig`
   exists **and** the server is not local, e.g.
   `ChannelState::iap_config().filter(|_| !ChannelState::server_root_url_is_local()).map(|cfg| Arc::new(IapState::new(&cfg)))`.
   Consider extracting the boolean decision into a small pure helper
   `fn iap_state_enabled(iap_config_present: bool, server_root_url: &str) -> bool
   { iap_config_present && !host_is_local(server_root_url) }` in `state.rs` so the
   full decision (both dimensions) is unit-testable, and have the gate call it. No
   other lines in `initialize_app` change; `IapManager`, `ServerApi`, and
   `managed_iap_mint` wiring are untouched and become inert via the `None` state
   as traced above.
4. **No new env var, flag, proto, or server change** in this repo. The contract is
   the local-URL classifier only.

*Cross-repo contract (for the warp-server sibling spec):* There is **no** IAP
opt-out env var or flag. The client keys IAP purely on the resolved
`server_root_url`. `oz-local` already exports
`OZ_LOCAL_SERVER_ROOT_URL=http://localhost:8080`, and the worker already passes
the server-URL override to the launched client, which lands as
`WARP_SERVER_ROOT_URL` before `initialize_app`. So warp-server needs to do
**nothing new for IAP** beyond what it already does for the server URL; its spec
covers only oz-dev PATH resolution in `resolve_oz_path()` and docs. The one
requirement warp-server must preserve: the launched task client's resolved server
URL must be a host in the local allowlist above (it is: `localhost:8080`).

*Open questions resolved:*
- *Does the client see the local URL at the IAP gate?* Yes — verified. The
  override is applied at `run()` (`lib.rs:762-767`) before `initialize_app`
  (`:1410`) builds `iap_state` (`:1483`). No material blocker; no restructuring
  needed. (Reported to the foreman as a positive finding.)
- *Is gating `iap_state` alone enough to fully disable IAP (including the runner
  WIF mint)?* Yes — `IapManager::is_enabled()`, `start_refresh` early-return, and
  the startup-auth gate all key off `state.is_some()` (traced above).
- *IPv6 loopback representation.* `::1` appears as `[::1]` from `Url::host_str()`;
  the classifier and its unit test pin the exact form the `url` crate yields.

*Assumptions to confirm at the spec-approval gate* (only option A/q1 was answered
explicitly by the requester; the following are my recommended defaults, adopted
per the foreman's instruction — confirm or correct at approval):
- **(q2) Server URL is the sole authority.** IAP stays enforced for every
  non-local URL and there is no env/flag that can override that. (Implied by
  choosing A; this spec assumes it.)
- **(q3) Local host set** = `{localhost, 127.0.0.1, ::1, host.docker.internal}`;
  everything else keeps IAP. Confirm this set is complete for your workflow (e.g.
  no LAN IP or `*.local` host is used to reach oz-local).
- **(q4) When IAP is skipped**, the client runs the normal user-auth flow against
  the local server — only the IAP pre-gate is removed, auth is **not** bypassed.
- **(q5) Scope (client side)** = the IAP-enablement decision + unit tests;
  staging IAP behavior unchanged; release channels cannot opt out. oz-dev PATH
  resolution and oz-local env/docs are the warp-server sibling spec.

*Risks / blast radius:*
- The change touches the auth/IAP startup path, but only the **construction** of
  `iap_state`; all downstream IAP machinery is unchanged and simply observes a
  `None` state for local URLs (the same state it already handles on non-IAP
  builds). Non-local URLs (staging/prod) are byte-for-byte unaffected.
- The classifier defaults to IAP-on for anything it does not recognize, so the
  worst-case failure mode of a classifier bug is "IAP still enforced" (a
  false negative that reproduces today's behavior), never "IAP disabled against
  staging".
- Headless/backend startup logic, no user-visible UI surface is added or changed
  (only the absence of an error toast in the local case): per `factory-verification`
  this is **not** a user-facing change, so no `computer_use`/visual proof is
  required — verification is the regression tests plus warp's documented checks.
  The end-to-end confirmation (client reaches local server with no IAP error) is
  a manual dev-machine step recorded below, since it needs the full local stack
  that the triage sandbox could not stand up.

*Validation & verification criteria* (must ALL pass before merge):
1. **Regression test — local host disables the decision (fails before, passes
   after).** A new unit test in `warp/crates/warp_core/src/channel/state_tests.rs`
   asserting `host_is_local` (and/or `iap_state_enabled`) returns the local
   verdict for each of `http://localhost:8080`, `http://127.0.0.1:8080`,
   `http://[::1]:8080`, and `http://host.docker.internal:8080`. Expressed against
   `iap_state_enabled(true, <url>)`, each must be `false` (IAP off). This test
   references functions that do not exist before the change, so it fails to
   compile/pass before and passes after. - verifies invariants #1, #2 (decision
   side).
2. **Negative test — non-local host keeps IAP enforced (the required negative
   test).** In the same test file, assert `iap_state_enabled(true, "https://staging.warp.dev")`
   is `true` and `host_is_local("https://staging.warp.dev")` is `false`; likewise
   for `https://app.warp.dev`. This guarantees IAP stays enforced for the real
   staging/prod server. - verifies invariant #3 and the core security requirement.
3. **Edge-case tests — classifier robustness.** Assert `host_is_local` returns
   `false` for an unparseable/garbage input (`"not a url"`) and for a host that
   merely contains a local substring but is not local (e.g.
   `https://localhost.evil.example.com` and `https://mylocalhost.dev`), and that
   port/scheme variations of a local host still classify local (e.g.
   `http://localhost` with no port). - verifies invariants #4 (pure function),
   #5, and that the allowlist is exact-match on host, not substring.
4. **No-config no-op.** Assert `iap_state_enabled(false, <any url>)` is `false`
   (a build with no `IapConfig` never enables IAP regardless of URL), covering the
   production/release path. - verifies invariant #5.
5. **Gate wiring is exercised.** Confirm `warp/app/src/lib.rs:1483` calls the new
   decision (either the `filter` on `server_root_url_is_local()` or
   `iap_state_enabled`) — verified by code review of the diff and by criterion 6
   compiling with the new call. (No heavy `initialize_app` integration test is
   required; the decision is fully covered by the pure-function tests above.) -
   verifies invariants #1, #3 at the call site.
6. **Repo checks pass.** From the `warp` repo root: `./script/format` (and confirm
   clean), `cargo clippy -p warp_core -p warp --all-targets --tests -- -D warnings`,
   and the affected crate tests — at minimum
   `cargo nextest run -p warp_core --no-fail-fast` covering
   `channel::state` (and `cargo nextest run -p warp` if `lib.rs` test coverage is
   added). New tests live in `state_tests.rs` (or a new `*_tests.rs`) per the
   repo's no-inline-test-modules rule. Run `./script/presubmit` before readying the
   PR; its CI is the full-suite backstop. - verifies the change compiles, lints,
   and does not break adjacent behavior.
7. **End-to-end (manual, dev machine — the original repro).** Build a local
   `oz-dev` binary; from `warp-server` run `./script/oz-local` (with the sibling
   PR's oz-dev resolution so `--oz-path` is not needed) with the local stack
   prerequisites satisfied; submit a task; confirm the launched client reaches
   `http://localhost:8080` and **no** "IAP credential refresh failed" error / IAP
   startup gate occurs, and that normal user auth proceeds. Then, as the negative
   E2E, point a Dev build at `https://staging.warp.dev` and confirm IAP is still
   established (unchanged). This step needs the full local stack the triage
   sandbox could not provision, so it is a reviewer/implementor dev-machine check.
   - verifies invariants #1, #2, #3, #7 end-to-end.
