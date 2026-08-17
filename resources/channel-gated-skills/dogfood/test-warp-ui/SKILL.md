---
name: test-warp-ui
description: >
  Guides testing Warp UI features and changes using the computer use tool.
  Use this skill only when the computer_use tool is available to the agent.
  Covers launching Warp and verifying UI behavior.
user-invocable: false
---

# Computer Use for Warp UI Testing

Use the `computer_use` tool to visually test that Warp looks and behaves as intended after UI changes.

## Running Warp

Launch Warp from the repository root. The exact command depends on which environment variable holds the API key:

- If `WARP_API_KEY` is already set, omit the flag entirely — the `--api-key` flag is bound to `WARP_API_KEY`, so Warp reads it automatically:

  ```bash
  cargo run --bin warp
  ```

- If the key is in `STAGING_USER_WARP_API_KEY` instead, pass it explicitly via the flag:

  ```bash
  cargo run --bin warp -- --api-key $STAGING_USER_WARP_API_KEY
  ```

Always pass `--bin warp` explicitly. That target builds the internal (dogfood) channel, which is the only channel that honors `--api-key` for the GUI app. A plain `cargo run` builds the OSS channel, which ignores the key and falls back to interactive onboarding.

Where the key is accepted, authenticating this way starts the app directly without interactive login prompts. In a cloud sandbox it currently is not — see "If the app is logged out" below before you plan a verification around it.

Initial builds may take several minutes; subsequent incremental builds are faster.

### Verify the launch is authenticated

The rendered window is the only reliable signal. Authenticated Warp opens straight to the terminal; unauthenticated Warp shows the logged-out onboarding/sign-in screen. Look at the window before you test anything.

The log distinguishes nothing. `Authenticating via pending API key` (`app/src/auth/auth_manager.rs`) is logged *before* the attempt, and neither outcome is logged after it; the IAP cache fast path (`crates/warp_server_client/src/iap.rs`) is silent on success as well. An absence of errors is not a successful login.

### If the app is logged out

In a cloud sandbox, expect it: **no way to launch an authenticated GUI there is known to work today.** Two independent walls, both reproduced against a real `cargo run --bin warp`.

- **The key is the wrong kind.** `WARP_API_KEY` in a cloud sandbox is a *service-account* key, and the GUI requires a user account. The app lands on onboarding with `Unauthorized: Expected a user account` and `invalid input syntax for type uuid: "serviceAccount:<uid>"`, carrying the key's own UID; `STAGING_USER_WARP_API_KEY` is usually not set there either. A genuine user-account key clears those errors, and then every authenticated call returns `403 Forbidden` for reasons not yet understood.
- **Staging dogfood gates the API-key login behind IAP.** Unlike the TUI (which authenticates immediately and resolves IAP out of band), the GUI withholds `--api-key`/`WARP_API_KEY` authentication until an IAP token is loaded (`authenticate_user_after_iap_access` in `app/src/lib.rs`). The sandbox self-mints one via Workload Identity Federation — a valid `OZ_RUN_ID` enables the runner-context mint path (`app/src/lib.rs`), which exchanges the injected `WARP_STAGING_IAP_BOOTSTRAP_JWT` for a token (`crates/warp_server_client/src/iap.rs`), no `gcloud` needed. That bootstrap JWT lives exactly 900 seconds from the start of the run. Past it the mint dead-ends (`Staging IAP access unavailable before startup user authentication`) and login is never attempted at all, so a run older than 15 minutes never reaches the key.

Settings > Account carries a "Staging IAP credentials" status widget (`app/src/settings_view/main_page.rs`), but Settings is unreachable from the onboarding screen — no menu bar, no gear icon, and Ctrl+Comma does nothing — so it cannot be read in the state that needs it.

Once a real launch attempt has landed on onboarding, falling back to the hardcode/mock path below is appropriate. Never describe a capture made against a mocked or hardcoded state as a live Cloud Mode (or other authenticated-surface) verification — say plainly that it's mocked.

## Testing Workflow

### 1. Hardcode or Mock Data (When Needed)

If you just need to verify that a specific UI looks correct, it can be useful to hardcode or mock data so the UI state is immediately reachable without navigating a full flow. This is optional — skip this step when testing end-to-end flows that should work naturally.

Examples of when to hardcode:

- **Conditional UI**: The feature only appears under certain conditions (e.g., a specific setting, a non-empty data set, an active subscription) — hardcode the condition so the UI always appears.
- **Feature flags**: The feature is behind a flag that isn't enabled yet — enable it directly.
- **Error states**: You want to test error handling UI — hardcode error responses or failure conditions.

Keep mocked changes minimal and focused — only change what's necessary to reach the UI state under test.

### 2. Invoke Computer Use

Call the `computer_use` tool with a task description that includes:

- The command to build and launch Warp from the repo root: `cargo run --bin warp` when `WARP_API_KEY` is set in the environment, or `cargo run --bin warp -- --api-key $STAGING_USER_WARP_API_KEY` when the key is in `STAGING_USER_WARP_API_KEY` instead
- Step-by-step instructions for navigating to the UI being tested
- **Specific observations to report**: describe exactly what elements, text, colors, layout, or states the tool should observe and describe back
- Do **not** include expected values in the task — the tool should report what it sees, not judge correctness

### 3. Verify Results

Compare the observations returned by `computer_use` against your expectations. If the UI doesn't match, investigate and adjust the code or mocks accordingly.

## Tips

- **Be specific in task descriptions**: Instead of "check if the dialog looks right," say "open Settings, click the General tab, and describe the text and layout of the first section."
- **Test one thing at a time**: Focused tests are easier to debug when observations don't match expectations.
- **Build before invoking**: Always confirm the build succeeds before calling `computer_use`. The tool cannot fix build errors.
