# Spec: Configurable last-tab window closing

Linear: [APP-5230](https://linear.app/warpdotdev/issue/APP-5230/setting-keep-window-open-when-closing-the-last-tab)

Baseline: `warpdotdev/warp` at `5688e06d9fd7a9b1530d28da1c1e2b295c37602e`

## Product

### Summary

Warp always closes a desktop window when its final tab is closed. Add a global setting that preserves this behavior by default but lets a user explicitly close the final tab while keeping the window open. The resulting window has no tabs, a visually blank content region, a persistent new-session control, and a faded keyboard hint in the tab bar.

### Key design choices

1. The new **Close window when all tabs are closed** setting defaults on, is globally persisted and synced, and appears in **Settings > Features > General** directly below **Quit when all windows are closed** where that macOS-only setting is present.
2. When the setting is off, explicitly closing the final tab produces a real zero-tab workspace rather than a placeholder tab or an automatically created session. Standard new-session entry points recover the window.
3. Last-tab drag is intentionally asymmetric with last-tab close: successfully dragging the only tab into another window still closes the source window, regardless of the setting, because the source window itself is the floating drag preview and restoring it empty would be visually surprising.

### Behavior

1. **Default behavior is unchanged in a close-capable host.** With **Close window when all tabs are closed** on, explicitly closing the final tab closes its window using the existing window-close and quit-warning behavior.
2. **The opt-in behavior removes the final tab without closing the window.** With the setting off, explicitly closing the final tab leaves the same window open with zero tabs and a visually blank main content region.
3. **The zero-tab tab bar remains actionable.**
   - The active tab presentation remains visible with its new-session `+` control.
   - It contains faded text reading **Press ⌘+T to create a new session** on macOS. On other platforms or with a customized binding, the displayed shortcut uses Warp's resolved new-tab keybinding while retaining the same sentence.
   - In horizontal-tabs mode, the hint occupies the empty tab-strip area immediately before the `+` control.
   - In vertical-tabs mode, the hint and `+` appear in the active vertical tab presentation. If the vertical-tabs panel is collapsed, the top header presents a compact zero-tab recovery strip containing both until a tab is created.
   - The hint is single-line and lower emphasis than normal tab text. At narrow widths, it ellipsizes and then hides before displacing the `+`, window controls, or existing right-side controls.
4. **The window remains a window, not a launcher.** Title bar, window controls, tab chrome, Settings access, command palette, menus, and existing sidebars remain rendered. Tab- or pane-dependent controls are disabled or show their existing no-active-content state; no control may retain or dereference the closed pane group. The blank main content region contains no message, button, terminal, launcher, or placeholder tab.
5. **All standard new-session entry points recover a zero-tab window.** The tab-bar `+`, the resolved New Tab shortcut (⌘T by default on macOS / Ctrl+T by default elsewhere), File-menu New Tab, Command Palette New Tab, and the new-session menu create the first real tab at index `0`, focus it, and restore normal workspace behavior. The keyboard hint is informational, not a separate button.
6. **Normal tab-close safety applies when the window will remain open.** Closing the final tab into the zero-tab state follows the same shared-session, long-running-process, and unsaved-code confirmation paths as closing any other tab. Cancellation leaves the tab and window unchanged. The final tab is eligible for the normal Undo Close behavior.
7. **Window-close safety remains unchanged when the window will close.** When the setting is on in a close-capable host, the final-tab path continues to defer to the existing window-close confirmation rather than presenting both a tab-close and a window-close confirmation.
8. **Explicit Close Window and Quit are unchanged.** They continue to close/quit and warn according to existing settings whether the window has tabs or is empty. Transitioning to zero tabs does not itself count as closing a window and must not trigger **Quit when all windows are closed**.
9. **Hosts without `ContextFlag::CloseWindow` can reach zero tabs.** Closing their final tab must no longer be a no-op. Because the host cannot close its containing window, final-tab close produces the zero-tab state regardless of the setting's value. The last-tab close affordance is available while a tab exists, and Close Window remains unavailable.
10. **Dragging the final tab remains window-closing.** If the only tab is successfully dragged or moved into another existing window, the target receives the tab and the source window closes, with both setting values and in `CloseWindow`-disabled contexts where the host supports the drag operation. A cancelled drag returns to the original one-tab state. The source must not flash or reappear as an empty window.
11. **Empty windows are not restored.** The preference persists and syncs, but a zero-tab window is omitted from session persistence. Relaunching Warp creates/restores the normal initial session instead of reconstructing an empty window.
12. **The setting is global.** It applies to all windows and has no per-window override.

### Out of scope

- Customizable empty-state copy, actions, artwork, or content.
- A per-window variant of the setting.
- Changes to explicit Close Window, Quit, **Quit when all windows are closed**, or their warning policy.
- Changing the successful single-tab cross-window drag workflow to preserve an empty source window.
- A tab-drag redesign or a separate floating preview window.
- Documentation-repository changes.

## Tech

### Current context

- `GeneralSettings` defines globally synced GUI lifecycle preferences including quit warnings, **Quit when all windows are closed**, and session restoration in `app/src/terminal/general_settings.rs:8` at the baseline commit.
- The General section conditionally inserts `QuitWhenAllWindowsClosedWidget`, and the widget dispatches a `FeaturesPageAction`, in `app/src/settings_view/features_page.rs:2774` and `app/src/settings_view/features_page.rs:4927`.
- `Workspace::remove_tab` treats `tabs.len() == 1` as a request to close the window and never removes the final `TabData`. `Workspace::close_tab` also makes a single tab a no-op when `ContextFlag::CloseWindow` is unavailable and unconditionally skips tab-level confirmation for the final tab in `app/src/workspace/view.rs:12067` and `app/src/workspace/view.rs:12377`.
- The Close Active Tab binding is available only for `Workspace_CloseWindow` or `Workspace_MultipleTabs`, so a single tab cannot be closed in a `CloseWindow`-disabled host (`app/src/workspace/mod.rs:1087`).
- The workspace currently asserts that zero tabs are invalid and unconditionally dereferences the active pane group during keymap-context construction and rendering (`app/src/workspace/view.rs:26369`, `app/src/workspace/view.rs:22026`, and `app/src/workspace/view.rs:26542`).
- New-tab insertion already has explicit empty-workspace branches that insert at index `0`, which can be retained and expanded (`app/src/workspace/view.rs:12802`).
- App-state collection already omits a window whose snapshot contains no tabs, matching the requested non-restoration behavior without a persistence schema change (`app/src/app_state.rs:385`).
- A single-tab cross-window drag uses the source window as its preview and closes it after handoff. That existing behavior should remain separate from the explicit tab-close path (`app/src/workspace/view.rs:28241` and `app/src/workspace/cross_window_tab_drag.rs:1225`).

### Design alternatives

- **Setting ownership — `GeneralSettings` selected over `TabSettings`.** `TabSettings` groups tab layout and placement, but this setting governs window lifecycle, must sit beside other window/app lifecycle settings, and needs the same global sync semantics as `GeneralSettings`. Define it in `GeneralSettings` with TOML path `general.close_window_when_all_tabs_closed`, default `true`, GUI surface, all-platform support, and global cloud sync respecting the user's sync preference.
- **True zero-tab workspace selected over a sentinel/placeholder tab.** A fake tab or pane would preserve existing active-pane invariants but would leak into tab counts, navigation, persistence, menus, and drag logic and would violate the requested empty tab bar. Keep `Workspace.tabs` genuinely empty and make active-tab access explicit at zero tabs.
- **Blank content branch selected over a dedicated launcher or automatic replacement session.** A launcher is more discoverable and an automatic session minimizes code changes, but both conflict with the requester's visually blank pane. Recovery stays in the tab bar and standard new-session actions.
- **Explicit close and drag remain separate.** Initially, uniform “last tab removed” behavior was considered. Investigation showed that a single-tab drag moves the source window as its preview. The requester deliberately chose to keep the existing source-window close after successful drag rather than snap an empty source window back to its pre-drag bounds.
- **No feature flag.** The default-on setting itself protects existing behavior, and the off state is the requested opt-in. A second rollout control would duplicate that gate without reducing the zero-tab implementation work.
- **Tab-bar hint selected over content-area copy.** A centered content hint was recommended for discoverability, but the requester explicitly chose the tab bar. The implementation must preserve that placement and degrade the hint before essential controls at narrow widths.

### Proposed changes

#### 1. Add the global setting and its discoverability surfaces

- Add `close_window_when_all_tabs_closed` to `GeneralSettings` with:
  - type `bool`;
  - default `true`;
  - `SupportedPlatforms::ALL`;
  - `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`;
  - GUI surface;
  - public TOML path `general.close_window_when_all_tabs_closed`.
- Add a `CloseWindowWhenAllTabsClosedWidget` to **Features > General** immediately after the conditional `QuitWhenAllWindowsClosedWidget` insertion. Its visible label is exactly **Close window when all tabs are closed**, and its search terms cover close/window/tab/last/empty/keep open.
- Add the matching `FeaturesPageAction`, toggle-and-save handler, settings telemetry, and local-only/sync icon state.
- Add Command Palette entries **Enable closing the window when all tabs are closed** and **Disable closing the window when all tabs are closed** using `ToggleSettingActionPair`, plus a context flag populated by `Workspace::add_toggle_setting_context_flags`.

#### 2. Make zero tabs an explicit workspace state

- Treat `tabs.is_empty()` as a supported state with `active_tab_index == 0` as a non-dereferenceable sentinel only. Introduce/use optional active-tab and active-pane-group accessors for render, focus, keymap, panel, navigation, telemetry, and action code that can run without a tab.
- Final-tab removal must perform normal tab cleanup:
  - detach/shut down the closing pane group only after confirmation;
  - unsubscribe from it;
  - prune its MRU and group state;
  - add it to Undo Close when requested;
  - clear stale tab selection, rename, focus, sidecar, panel, synchronized-input, and active-session state;
  - reset `active_tab_index` to `0`;
  - save app state and notify rendering.
- Do not create an invisible pane group, placeholder terminal, or special `TabData`.
- When the first tab is added to an empty workspace, rebuild active-tab-derived panel/focus/session state through the normal activation path and guarantee an ungrouped insertion at index `0`.

#### 3. Separate “close the window” from “remove the final tab”

- For explicit final-tab close, calculate whether the action can and should close the window:
  - `close_window_when_all_tabs_closed == true` **and** `ContextFlag::CloseWindow` enabled → use the existing window-close path;
  - otherwise → use the normal confirmed tab-removal path and enter zero tabs.
- Skip tab-level confirmation only in the first branch where window close will actually run. The keep-open branch must not set `skip_confirmation` merely because the tab is final.
- Replace the Close Active Tab binding predicate with one based on the presence of at least one tab, not window-close capability. Add a `Workspace_NoTabs` (or equivalently named) keymap context and use it to hide/disable Close Tab and all active-session commands in the empty state while leaving New Tab and global window commands available.
- Preserve the cross-window drag cleanup branches. A successful last-tab handoff continues to return/handle `CloseSourceWindow`; it must not call the new explicit-close zero-tab transition. Add regression coverage so future cleanup refactors do not unify these paths.

#### 4. Render the zero-tab state safely

- Branch before `render_banner_and_active_tab` and before any active-pane-dependent panel rendering. Paint only the normal workspace background in the main content slot.
- Keep global window chrome and sidebars rendered. Tab-scoped panel models must be cleared or rendered without an active pane group; stale closed pane content is forbidden.
- Render the zero-tab hint and `+` in the active tab presentation as specified in Product behavior #3. Use the existing resolved new-tab binding display helper so custom and platform bindings are represented correctly.
- Do not render tab slots, an active-tab indicator, tab menus, tab overflow derived from tab contents, or an active content banner when there are no tabs.
- Ensure both horizontal and vertical tab layouts, including collapsed vertical-tabs mode and narrow windows, follow the same recovery contract.

#### 5. Preserve non-restoration and lifecycle behavior

- Retain the existing app-state rule that drops snapshots with no tabs. Add a regression test proving an empty workspace contributes no restorable window while the setting value itself persists.
- Do not alter `on_should_close_window`, `on_should_terminate_app`, or `quit_on_last_window_closed`. An explicit close of an empty window continues through those existing callbacks.
- Ensure `handle_reopen`, workspace registries, window titles, synchronized input, and focus change notifications tolerate no active pane group.

### Open questions resolved

1. **Empty content:** visually blank; no dedicated empty-state component or button.
2. **Recovery UI:** the tab bar stays visible with `+`; the faded shortcut hint lives inside the tab bar, not the content region.
3. **New-session entry points:** every standard New Tab path must work from zero tabs.
4. **Restart:** an empty window is not restored; relaunch seeds/restores a normal session.
5. **Confirmations:** normal tab/session confirmation applies only when the final tab is removed without closing the window; explicit window/quit behavior is unchanged.
6. **Cross-window drag:** the later geometry discussion deliberately superseded the initial preference for uniform removal behavior. Successful final-tab drag always closes the source window because that window served as the moving preview.
7. **`CloseWindow`-disabled hosts:** they are in scope and may reach zero tabs even while the global setting is on, because they cannot honor its window-close side.
8. **Scope:** the preference is global, persisted, and synced; no per-window override or customizable empty state.
9. **Chrome:** global window chrome remains; active-session actions must fail closed rather than panic or operate on stale content.
10. **Prototype:** implementation must provide a Dogfood/feature-branch build the requester can try, plus computer-use video proof.

### Risks and mitigations

- **Active-tab invariant is pervasive.** Rendering, keymap contexts, focus, synchronized input, panels, window titles, and telemetry currently dereference the active pane group. Mitigate with an explicit optional active-tab boundary, a dedicated no-tabs context, zero-state render branches, and regression tests that exercise actions while empty.
- **Closing can lose process or editor state.** The old final-tab path relied on window-close confirmation. Mitigate by selecting the confirmation owner from the actual outcome and testing shared-session, long-running-process, unsaved-code, cancel, and confirm cases.
- **Cross-window code could accidentally use the new removal path.** Mitigate with separate explicit-close and transfer APIs plus tests proving a successful last-tab handoff closes the source for both setting values.
- **A fake or stale pane can leak into persistence.** Mitigate by requiring a true empty `tabs` vector, clearing pane-group references, retaining snapshot filtering, and testing relaunch behavior.
- **The tab-bar hint can crowd controls.** Mitigate with low-priority shrink/ellipsis/hide behavior and visual verification at normal and minimum supported widths in horizontal, vertical, and collapsed-vertical layouts.
- **Platform behavior diverges.** macOS can remain alive with no windows while Linux/Windows normally terminate on final-window close, and web/link hosts cannot close windows. Mitigate by testing the effective-close decision independently from the global preference and using cross-platform CI as the backstop.

## Validation and verification criteria

All criteria must pass before merge.

1. **Setting contract and compatibility (Behavior #1, #12):** a unit/settings test proves `general.close_window_when_all_tabs_closed` defaults to `true`, persists, globally syncs under the normal sync preference, and the Settings switch plus Command Palette enable/disable actions mutate the same value. Existing user configurations with no key retain current close-capable desktop behavior.
2. **Settings placement and copy (Behavior #1):** a running GUI shows **Close window when all tabs are closed** in **Settings > Features > General**, directly below **Quit when all windows are closed** on macOS and in the corresponding position after the conditional slot on other platforms. Settings search finds it using “close last tab” and “keep window open.”
3. **Default explicit-close path (Behavior #1, #7):** with the setting on in a `CloseWindow`-capable desktop window, closing the final tab from the tab close button/menu and the resolved Close Tab shortcut closes the window. Existing window/quit warning behavior is exercised and no transient zero-tab frame is shown.
4. **Opt-in explicit-close path (Behavior #2, #6):** add a regression test in `app/src/workspace/view_tests.rs` that fails on the baseline and passes after the change: set the preference off, close the final ordinary tab, and assert the window close path is not requested, `tab_count() == 0`, active-tab-derived state is cleared, and the closed tab is placed on the normal Undo Close stack.
5. **Confirmation ownership (Behavior #6, #7, #8):** automated tests cover a final tab with a shared session and the available unsaved/long-running summary seam:
   - setting off → tab-level confirmation appears; Cancel keeps one tab; Confirm produces zero tabs;
   - setting on in a close-capable host → tab-level confirmation is skipped and existing window-close confirmation owns the decision;
   - explicit Close Window and Quit are unchanged from an empty window.
6. **Zero-tab UI (Behavior #3, #4):** computer-use verification shows a visually blank main content region, no fake tab/session, the persistent `+`, the faded shortcut hint inside the tab bar, and usable global chrome. It covers horizontal tabs, vertical tabs, collapsed vertical tabs, and a narrow window where the hint yields before essential controls.
7. **Zero-tab action safety (Behavior #4):** while empty, exercise tab navigation, Close Tab, pane/session commands, sidebar controls, Settings, menus, Command Palette, window focus changes, and synchronized-input/context generation. No panic, stale pane operation, phantom tab, or enabled active-session action occurs; global actions remain available.
8. **Recovery through every standard entry point (Behavior #5):** parameterized workspace/integration coverage starts from zero tabs and separately creates a first session through the `+`, resolved New Tab shortcut, File-menu New Tab, Command Palette New Tab, and new-session menu. Each produces exactly one focused, ungrouped tab at index `0`, clears the hint, and restores normal focus, title, panel, and session state.
9. **Undo Close recovery (Behavior #6):** from zero tabs, Undo Close restores the closed final tab as the sole focused tab without creating a second default session.
10. **`CloseWindow`-disabled host (Behavior #9):** a context-controlled test proves Close Tab is offered while one tab exists, final-tab close reaches zero tabs with the preference both on and off, Close Window remains unavailable, and all recovery paths work. Close Tab is unavailable after reaching zero.
11. **Cross-window asymmetry (Behavior #10):** cross-window regression coverage proves that a successful only-tab handoff closes the source window with the preference both on and off, adds exactly one tab to the target, does not duplicate or lose the transferred pane group, and never renders the source as empty. A cancelled drag restores the original one-tab source.
12. **Persistence and restart (Behavior #11):** an app-state test proves zero-tab windows are omitted from `AppState.windows`; restart/session restoration creates or restores a normal session rather than an empty window; the global preference retains its saved value.
13. **Adjacent multi-tab behavior:** existing tests for closing horizontal/vertical tabs, active-neighbor selection, tab groups, pinned tabs, Close Other Tabs, Close Tabs Right/Below, tab drag, and session-close confirmation continue to pass unchanged.
14. **Deterministic test gate:** `cargo nextest run -p warp` passes for the touched app package. Any added integration test for the GUI flow also passes through the repository's `crates/integration` harness. PR CI is the full-workspace/cross-platform backstop for this M-sized app-only change.
15. **Repository quality gates:** `./script/format` and the repository-prescribed `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` complete successfully before the PR is promoted from draft.
16. **Hands-on prototype and visual proof:** implementation supplies a runnable Dogfood/feature-branch build for the requester to try. Computer use records and attaches a video that demonstrates, in one coherent flow:
   - the default-on setting closing the final tab's window;
   - toggling the setting off in the specified Settings location;
   - final-tab confirmation and the resulting zero-tab UI;
   - creating the first replacement session with both the `+` and ⌘T;
   - a successful drag of an only tab into another window closing its source despite the setting being off.
   The video is attached to both the Linear task and the reused implementation PR.
