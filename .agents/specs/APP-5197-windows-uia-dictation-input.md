# Spec: Windows UI Automation input provider for third-party dictation (APP-5197)

- Linear issue: [APP-5197](https://linear.app/warpdotdev/issue/APP-5197/superwhisper-voice-transcription-does-not-insert-into-warp-terminal)
- GitHub issue: https://github.com/warpdotdev/warp/issues/10103
- Originating thread: https://warpdev.slack.com/archives/C0BDQDW8V5E/p1785969733196699
- Repo: `warpdotdev/warp` — estimate XL
- Code references are pinned to `fe6b6755cb69c5f331e7a58c54cca2504da9ff1f` (master at spec time).

This is a large, cross-cutting **native-Windows platform** change. It is Windows-only
and **not reproducible on the Linux triage/spec sandbox** (no Windows host, no
Superwhisper, and Warp's UI Automation surface only exists on Windows). Root cause was
confirmed by hands-on code investigation; end-to-end verification is manual on a Windows
build (see Validation).

## PRODUCT

### Summary
On Windows, third-party voice-dictation tools (Superwhisper, and any UI Automation
based tool) cannot automatically insert transcribed text into Warp's focused input.
Warp's `warpui` framework paints its own input region and exposes **no** Windows UI
Automation (UIA) provider, so a dictation tool's automatic insertion (UIA
`ValuePattern.SetValue` / `TextPattern`, or a `WM_CHAR` sent to a focused edit control)
has no target and is silently dropped. The transcript still lands on the clipboard, so a
manual `Ctrl+V` works — but automatic insertion does not. Windows Terminal hosts a
standard OS edit control with a TextPattern provider, so the same dictation lands there.
macOS is unaffected because it implements native accessibility.

The desired outcome: on Windows, dictation/automation tools can locate Warp's focused
text input and write text into it automatically, without the user pressing `Ctrl+V`.

### Key design choices
1. **Adopt the `accesskit` crate** (with its Windows UIA backend, wired through the
   winit event loop) rather than hand-rolling a UIA COM provider — chosen by the
   requester and because it removes almost all of the UIA/COM boilerplate and gives a
   reusable a11y foundation. (Alternatives documented in Tech.)
2. **Minimal provider depth, broad surface.** Expose only the a11y nodes needed for
   dictation/automation *insertion* into the focused text field — a focused text node
   with `Value`/`TextPattern` and an action to set/insert text. Do **not** build a full
   screen-reader accessibility tree (buttons, lists, navigation) in this change. Apply
   this shallow provider to **all `warpui` text-input surfaces**, not just the terminal
   command input. (Reconciles the requester's "minimal scope" + "all text fields"
   answers.)
3. **Cover both delivery paths.** Ship the UIA provider **and** confirm/harden the
   synthesized-keystroke path (`SendInput`/`WM_CHAR` → `TypedCharacters`), because
   Superwhisper's "Simulate keystrokes" mode was already ON for the reporter and still
   failed. Route all programmatic insertion into Warp's existing typed-characters / paste
   insertion path so behavior matches real typing.

### Behavior (numbered, testable invariants)
From the user's / assistive-tool's point of view on **Windows**:

1. **Default happy path.** With Superwhisper "Simulate keystrokes" enabled and Warp's
   terminal command input focused, speaking a phrase inserts the transcribed text into
   the Warp input automatically — no manual `Ctrl+V` required.
2. **UIA provider exists on the focused input.** When any `warpui` text-input surface is
   focused, a UIA client (Windows Narrator, `Inspect.exe`, or Accessibility Insights for
   Windows) reports a focused element that exposes a text value and supports programmatic
   text entry (a `ValuePattern` and/or `TextPattern` with a settable value). Before this
   change no such element exists.
3. **Programmatic insertion writes into the field.** A UIA client that calls the value /
   text set-or-insert action on the focused Warp input causes that text to appear in the
   input, positioned at the caret, identical to typing it.
4. **All text-input surfaces, not just the terminal.** Invariants 2–3 hold for every
   `warpui` text-input surface — the terminal command input, the Agent/AI compose input,
   the in-app search/filter input, and the code editor — because they share the
   `EditorView` text-input component. Whichever surface currently has keyboard focus is
   the one the provider targets.
5. **Synthesized-keystroke mode also inserts.** With a dictation tool configured to send
   synthesized keystrokes (`SendInput`) while Warp is focused, the characters are
   inserted into the focused input (this path already partially works via the winit
   `TypedCharacters` handling; this change confirms and, if a gap is found, hardens it).
6. **No regression to existing input.** Manual typing, IME composition, and manual
   `Ctrl+V` paste into every affected field continue to work exactly as before.
7. **Windows-only, no cross-platform regression.** macOS and Linux input and
   accessibility behavior are unchanged. The new provider code is gated so it only
   compiles/activates on Windows.
8. **No-op when unfocused.** When no Warp text input is focused (or Warp is not the
   foreground window), programmatic insertion targeted at Warp does nothing and does not
   crash — matching how a native edit control behaves when it lacks focus.

### Explaining the "works for colleagues, not me" discrepancy
The reporter noted colleagues use the same Warp-on-Windows workflow without needing
`Ctrl+V`. The most likely explanation, grounded in the code: Warp today has **no UIA
target**, but the winit keyboard path **does** accept synthesized characters
(`event_loop/mod.rs` `KeyboardInput` → `TypedCharacters`, including a fallback for
`SendInput`-injected `Key::Unidentified` characters at
`crates/warpui/src/windowing/winit/event_loop/mod.rs:1308` @ `fe6b675`). So a
Superwhisper/OS configuration that delivers via **synthesized keystrokes to the focused
window** already reaches Warp (colleagues), whereas a configuration that delivers via
**UIA `SetValue` / paste-to-focused-control** finds no target and is dropped (reporter).
Focus/foreground timing (Warp not being the foreground window at insertion time) can also
differ per machine. This change removes the discrepancy by making the UIA path work for
everyone; hardening the keystroke path is the secondary safety net. The implementor
should record the concrete difference they observe on the reporter's configuration during
verification.

## TECH

### Context — how this works today
- `warpui` renders its own input region; it does not host a native OS text control. See
  the module docs in `crates/warpui_core/src/accessibility.rs:1` @ `fe6b675`
  ("Because Warp uses its own rust UI framework (warpui), we don't benefit from the
  built-in VoiceOver integration…").
- `warpui` already models per-view accessibility content — `AccessibilityContent`
  (`value`, `help`, `role`) with roles like `TextareaRole` / `TextfieldRole`
  (`crates/warpui_core/src/accessibility.rs:53` and `:206` @ `fe6b675`). The focused
  view's content (e.g. `value = "Command Input"`) is what a provider node would surface.
- On **Windows/Linux/wasm** the winit delegate accessibility hooks are no-ops:
  `set_accessibility_contents` (`crates/warpui/src/windowing/winit/delegate.rs:545`) does
  nothing and `is_screen_reader_enabled` (`:566`) returns `None` @ `fe6b675`.
- On **macOS** these route to native AppKit accessibility
  (`crates/warpui/src/platform/mac/delegate.rs:302` and `:173` @ `fe6b675`), which is why
  the bug is Windows-only.
- There is **no** UIA provider anywhere: a repo-wide grep for `accesskit`,
  `WM_GETOBJECT`, `IRawElementProviderSimple`, `ITextProvider`, `IUIAutomation`, `UIA_`
  returns zero matches. The `windows` crate is already a Windows dependency with several
  `Win32_*` features (`crates/warpui/Cargo.toml:211` @ `fe6b675`), but no
  `Win32_UI_Accessibility`.
- Windows window/HWND handles are obtained from winit via `raw_window_handle`
  (`RawWindowHandle::Win32`), as in `crates/warpui/src/windowing/winit/windows/window_ext.rs:34`
  @ `fe6b675`. All `WindowEvent`s flow through the shared winit event loop
  (`crates/warpui/src/windowing/winit/event_loop/mod.rs` @ `fe6b675`).
- The insertion primitive for the input is `EditorModel::insert` / `EditorView::user_insert`,
  driven by the `TypedCharacters` event and by paste
  (`app/src/editor/view/mod.rs:1980` `insert_char`/`user_insert`, `:2012` `editor_model.insert`,
  `:2539` `paste` @ `fe6b675`). Manual `Ctrl+V` uses this same path, independent of any
  UIA surface, which is why it works today.

### Design alternatives (per decision point)
- **UIA integration: `accesskit` vs. hand-rolled COM provider.**
  - *`accesskit` (chosen — requester preference).* Cross-platform a11y crate with a
    Windows UIA backend; builds an accessibility tree from plain Rust structs and handles
    the COM/`WM_GETOBJECT`/activation plumbing. Pros: far less unsafe COM code; reusable
    for future macOS/Linux a11y; maintained upstream. Cons: new dependency; must fit
    Warp's `winit` version and its bespoke event loop; conceptually a full a11y-tree model
    even though we only need a shallow node.
  - *Hand-rolled UIA provider (rejected).* Implement `IRawElementProviderSimple` +
    `IValueProvider`/`ITextProvider` directly and answer `WM_GETOBJECT` on the winit HWND.
    Pros: minimal surface, no new dependency, full control. Cons: substantial, error-prone
    `unsafe` COM/UIA code to write and own; reinvents what accesskit already ships.
- **How to wire accesskit into winit: `accesskit_winit` adapter vs. raw `accesskit_windows`.**
  - *`accesskit_winit` (recommended).* Integrates with the winit event loop and handles
    the `WM_GETOBJECT` hand-off and the UIA activation handshake for us. Risk to resolve:
    version compatibility with Warp's pinned `winit` and its custom event pump — the
    implementor must confirm the adapter can be driven from
    `crates/warpui/src/windowing/winit/event_loop/mod.rs` (feeding it window events and
    tree updates), or fall back to `accesskit_windows` driven directly from the HWND.
  - *`accesskit_windows` directly.* More control, but we re-own the activation/`WM_GETOBJECT`
    wiring. Use only if the winit adapter cannot be integrated cleanly.
- **Provider depth: shallow focused-input node vs. full tree.** Shallow chosen — one
  (or a tiny fixed set of) node(s) representing the focused text input with a settable
  value, no navigable tree. Keeps scope to dictation/automation insertion; a full
  screen-reader tree is explicitly a future follow-up.
- **Insertion routing: reuse `TypedCharacters`/`user_insert` vs. a new insertion API.**
  Reuse the existing path so programmatic insertion is indistinguishable from typing and
  inherits autosuggestion/undo/selection behavior. A new bespoke insertion API is
  rejected as duplicative and regression-prone.
- **Surfaces: all `warpui` text fields vs. terminal only.** All (requester answer). Made
  cheap by targeting the shared `EditorView` component generically and always pointing the
  provider at the currently focused editor, rather than wiring each surface individually.

### Proposed changes
1. **Dependencies (Windows target).** Add `accesskit` (+ `accesskit_winit` or
   `accesskit_windows`) under the `cfg(target_os = "windows")` dependency block in
   `crates/warpui/Cargo.toml`. Pin versions compatible with the workspace `winit`. Do not
   add these deps for macOS (native a11y) or, for this change, Linux.
2. **Provider module.** Add a Windows-only accessibility provider (e.g.
   `crates/warpui/src/windowing/winit/windows/accessibility.rs`, or a shared
   `windowing/winit/accessibility.rs` gated to Windows) that:
   - Builds an accesskit tree containing a focused text-input node whose value reflects
     the focused view's `AccessibilityContent` and whose role is text/editable.
   - Registers the accesskit adapter against the winit window/HWND so UIA `WM_GETOBJECT`
     requests are answered.
   - Handles accesskit `ActionRequest`s for setting/inserting/replacing text and routes
     them to the focused editor's insertion path (below).
3. **Wire the winit delegate a11y hooks.** Implement `set_accessibility_contents` and
   `is_screen_reader_enabled` on the winit delegate
   (`crates/warpui/src/windowing/winit/delegate.rs:545`, `:566`) for Windows so focus /
   content changes push tree updates to the adapter. Keep the wasm/Linux behavior a no-op.
4. **Event-loop integration.** Feed the accesskit adapter from the winit event loop
   (`crates/warpui/src/windowing/winit/event_loop/mod.rs`) — window init, focus changes,
   and any events the adapter needs — and translate its `ActionRequest`s into a warpui
   event (reuse/extend `TypedCharacters`, or a new internal insert event) delivered to the
   focused window's callbacks. Route into `EditorView::user_insert` / `EditorModel::insert`
   (`app/src/editor/view/mod.rs`) so insertion matches typing.
5. **Harden the synthesized-keystroke path (secondary).** Confirm the existing
   `KeyboardInput` → `TypedCharacters` path (incl. the `Key::Unidentified` `SendInput`
   fallback at `event_loop/mod.rs:1308`) delivers dictation keystrokes when Warp is
   focused; fix any gap found. If no gap is found, record that finding in the PR and
   verification notes rather than changing code.
6. **Windows gating.** All new UIA/accesskit code is behind `cfg(target_os = "windows")`
   (or `cfg(windows)`), so macOS and Linux builds and behavior are unchanged.

### Open questions resolved
- **accesskit crate/winit version compatibility** — assumption: a compatible `accesskit`
  (+ `accesskit_winit`) exists for the workspace `winit`; the implementor confirms exact
  versions during implementation and falls back to `accesskit_windows` driven from the
  HWND if the winit adapter cannot integrate. Reviewer to confirm the chosen wiring.
- **Exact set of "all `warpui` text fields"** — resolved to "the shared `EditorView`
  text-input component, targeting whichever instance is focused," which structurally
  covers terminal command input, Agent/AI compose input, search input, and the code
  editor. The implementor enumerates the concrete focused-editor surfaces; any surface
  that does not use `EditorView` is out of scope for v1 and noted on the PR.
- **Linux (also winit).** Out of scope for v1 (requester chose the minimal Windows fix).
  If `accesskit_winit` is added in a cross-platform way, Linux behavior must be verified
  unchanged or the provider explicitly gated to Windows. Assumption: gate to Windows.
- **Full screen-reader support (Narrator navigation).** Out of scope; this change only
  guarantees the focused input node + programmatic insertion, not a navigable tree.
- **"Works for colleagues, not me."** Resolved as a delivery-mode/focus difference (see
  Product); no separate code fix beyond covering both delivery paths. The implementor
  records the concrete observed difference during Windows verification.

## Validation & verification criteria (must ALL pass before merge)

Because there is no Windows runner in the sandbox, the end-to-end criteria are **manual
on a Windows dogfood build, performed by the requester** (their choice). Each manual
criterion states exactly what must be observed to pass. Automated criteria run in normal
CI/local dev.

1. **Original report is fixed (manual, Windows).** On a Windows dogfood build of this
   branch, with Superwhisper "Simulate keystrokes" enabled and the Warp terminal command
   input focused, dictating a phrase inserts the transcribed text automatically, with **no
   manual `Ctrl+V`**. Pass evidence: a short screen recording (or the requester's explicit
   confirmation) showing dictated text appearing in the Warp input without `Ctrl+V`.
   (Verifies Behavior #1.)
2. **UIA provider present on the focused input (manual, Windows).** Using `Inspect.exe` or
   Accessibility Insights for Windows, focusing each affected `warpui` surface shows a
   focused element exposing a text value with `ValuePattern` and/or `TextPattern`
   (settable). Pass evidence: an Inspect.exe screenshot per surface showing the pattern(s).
   (Verifies Behavior #2, #4.)
3. **Programmatic insertion works (manual, Windows).** From a UIA client (Narrator input,
   Inspect.exe, or a small UIA harness) invoking the value/text set-or-insert action on the
   focused Warp input causes the text to appear at the caret. Pass evidence: before/after
   screenshots or recording. (Verifies Behavior #3.)
4. **All target surfaces covered (manual, Windows).** Criteria 2–3 pass for the terminal
   command input, the Agent/AI compose input, the search input, and the code editor. Any
   surface deliberately excluded is listed on the PR with a reason. (Verifies Behavior #4.)
5. **Synthesized-keystroke mode inserts (manual, Windows).** With a dictation tool in
   synthesized-keystroke (`SendInput`) mode and Warp focused, dictated text is inserted.
   The PR notes whether any code change was needed or the path already worked. (Verifies
   Behavior #5, and documents the discrepancy.)
6. **No input regression (manual, Windows + automated).** Manual typing, IME composition,
   and manual `Ctrl+V` still work in every affected field on Windows. Existing editor /
   terminal-input unit tests still pass:
   `cargo nextest run --no-fail-fast -p warp_app` (and any `editor`/`warpui` package tests
   touched). (Verifies Behavior #6.)
7. **No cross-platform regression (automated).** The workspace builds on all platforms and
   the new UIA code compiles only on Windows. Confirm a Windows build/CI job compiles the
   new `cfg(windows)` code (e.g. the repo's Windows CI, or `cargo check` targeting Windows
   / a Windows CI run on the PR); macOS and Linux builds are unaffected. (Verifies
   Behavior #7.)
8. **Unfocused / no-target is a safe no-op (manual or unit, Windows).** Programmatic
   insertion when no Warp input is focused does nothing and does not crash. Where the
   harness allows, cover the tree-building / action-routing logic (which is
   platform-independent enough to unit test) with a test asserting an insert action routes
   to `user_insert` only when a focused editor exists. (Verifies Behavior #8.)
9. **Regression/coverage test added.** Add at least one automated test around the new,
   testable logic (accesskit tree construction from `AccessibilityContent`, and/or the
   action-request → insertion routing) that fails before the change and passes after.
   Name it in the PR. The COM/`WM_GETOBJECT` surface itself is verified manually
   (criteria 2–3); this criterion covers the Rust-side logic that can be tested without a
   live UIA client. (This change alters user-facing input behavior, so it is **not** a
   testing-exempt category.)
10. **Repository gate passes (automated).** `./script/format` (max width 100),
    `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`, and the
    scoped test suites above all pass, per the repo's documented checks in `AGENTS.md`. The
    PR's full CI (including any Windows job) is the full-suite backstop.
11. **Changelog + PR hygiene.** The PR carries a `CHANGELOG-BUG-FIX:` line describing the
    Windows dictation fix (warp-only requirement), an Originating thread line, and the
    `factory-agent` metadata block.
