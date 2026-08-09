# PRODUCT: Multi-Level Orchestration in the Warp TUI (proposal)
Linear: [CODE-1944 — Design proposal: multi-level orchestration UI in the Warp TUI](https://linear.app/warpdotdev/issue/CODE-1944/design-proposal-multi-level-orchestration-ui-in-the-warp-tui)
Baseline it amends: [specs/code-1822-tui-orchestration-tab-bar/PRODUCT.md](../code-1822-tui-orchestration-tab-bar/PRODUCT.md)
GUI reference: [specs/orch-pill-bar-web/PRODUCT.md](../orch-pill-bar-web/PRODUCT.md), `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs`
Status: **proposal — not approved, not a plan of record.** Nothing here should be built before sign-off on the decisions in "Decisions that need the requester".

## Why a new spec directory rather than an edit in place
`specs/code-1822-tui-orchestration-tab-bar/PRODUCT.md` is the record of behavior that **shipped**. This document is an unapproved proposal that would supersede roughly fifteen of its clauses; folding it in would make it impossible to tell approved behavior from a pitch. It lives beside CODE-1822 in the same `code-1822-tui-*` family, names every clause it would replace, and should be merged back into the baseline spec if and when it is approved and built.

## Orientation: what the TUI orchestration UI looks like today
The premise in the request — "we haven't built out the UI yet for the TUI" — needs one correction. The TUI already ships a substantial orchestration UI, built under CODE-1822. What is missing is only the **multi-level** dimension, and only in the *presentation* layer. The behavior underneath is already multi-level: `FeatureFlag::MultiLevelOrchestration` is on in `DOGFOOD_FLAGS`, grandchildren get real retained sessions, they are restored on startup, and `kill_descendant_agents` unwinds nesting deepest-first. Only the rendering is flat.

There are three orchestration surfaces in `crates/warp_tui/src/`.

**1. The permission card** (`orchestration_block.rs`, `orchestration_block/render.rs`). Rendered exactly as below (real spans; the leading `■` is the attention glyph, agent glyphs are the deterministic identity palette from `orchestrated_agent_identity_styling.rs`):

```
■ Can I start additional agents for this task?
   Agents (3):
   ⊹ researcher  •  ⟡ implementer  •  ✶ reviewer

   Location: Local  •  Harness: Warp  •  Model: Default model

Enter to accept  Ctrl + E to edit Ctrl + C to reject
```

**2. The agent tab bar** (`orchestration_tab_bar.rs` over the generic `tab_bar.rs`). One row, always: a `Agents:` label, a fixed `orchestrator` main tab, a `|` divider, then one tab per navigable child. `Shift+↑` from the input focuses it; `Tab`/`←`/`→` switch sessions immediately.

```
   Agents:   orchestrator | ● researcher   ⟡ implementer   ■ reviewer
```

One glyph precedes each child label, not two: `orchestration_tab_icon` shows the **status** glyph while the child is live (`●` running/waiting, `■` blocked) and swaps to the child's **identity** glyph once it reaches a terminal state (`⟡`, `✶`, `⊹`, …). There is deliberately no `✓` in the bar. Labels cap at 20 display cells with `...` truncation; `←`/`→` arrows page the child region. The `orchestrator` tab carries no glyph at all.

**3. Inter-agent messages** (`agent_message.rs`) render in the transcript as collapsed-by-default disclosures: status glyph, identity glyph, sender name, body preview.

### The multi-level gap, precisely
`orchestration_model.rs:194` builds the snapshot from `descendant_conversations_in_pill_order(history, root)` — **every** descendant at **every** depth, flattened into one row. A grandchild renders as a peer of a child with no cue about parentage:

```
   Agents:   orchestrator | ● researcher   ● crawler   ● indexer   ⟡ implementer
                            └─ child ──┘   └─ these two are researcher's children ─┘
```

Consequently: no breadcrumbs (the GUI has `breadcrumb_ids`), no subtree rollup badge (the GUI has `loaded_subtree_rollup`), no aggregated status on the `orchestrator` tab (the GUI has `aggregated_orchestrator_status`), and `FeatureFlag::MultiLevelOrchestration` has **zero references** anywhere in `crates/warp_tui`, so the card never carries the GUI's "These agents may start their own child agents" disclosure.

### Two corrections to the triage write-up
- **The `?` overlay does not omit orchestration.** `terminal_session_view/state.rs:631-639` already emits an `Orchestration` section (`Shift+↑ navigate to agents`), gated on `orchestration_available`. It was absent from the captures because no orchestration tree existed in that session. The discoverability problem is narrower than reported and is addressed in §7.
- **A nested permission card can never render today.** `run_agents.rs:446-451` auto-executes `run_agents` unconditionally for any child conversation, because a child lives in a background session where a card would be invisible and would hang the run. So the multi-level disclosure only ever matters on the **root** card, which materially shrinks §6.

## Summary of the proposal
The bar becomes a **drill-down** bar with an explicit descend and an implicit ascend, mirroring the GUI's information architecture (`child_conversations_in_pill_order` against an anchor) but not its affordances. Concretely:

- The row renders **one level at a time**: the anchor tab, then that anchor's direct children.
- The anchor's ancestors render as compact, selectable **breadcrumb chips** to the left of the anchor. Selecting one ascends; that is the whole ascend mechanic, so no new "go up" key is needed.
- A child that is itself an orchestrator gets a trailing **`▸N` group badge** and a leading glyph that reflects its **rolled-up subtree** status.
- `←`/`→` stay **within the level**. `Tab`/`Shift+Tab` keep today's **tree-wide** walk, re-anchoring the bar as they land. `Enter` descends. Nothing else changes.
- At orchestration depth 1 — which is every session today — **the bar looks exactly as it does now**, plus a status glyph on `orchestrator` and a `▸N` badge on any child that spawned children.
- A defined **degradation ladder** keeps the bar useful down to 56 columns by shedding chrome (`Agents:`, then breadcrumb labels, then the anchor label) before it sheds child labels.

## Goals
- Make parentage legible in a single-row cell grid without a second row, a tree view, or a modal.
- Keep every agent in the tree reachable in a bounded number of keystrokes, as it is today.
- Reuse the GUI's canonical ordering, aggregation, and rollup helpers rather than inventing TUI-only policy.
- Stay legible and operable at 60 columns.
- Change nothing at depth 1.

## Non-goals
1. **Hover detail cards.** The GUI's 300ms hover card (working directory, task summary, harness chip, PR branch) has no terminal analogue and **nothing replaces it**. In the TUI, switching to an agent *is* one keystroke, and its session shows strictly more than the card would. A hover card would be a worse version of a cheaper action.
2. **Per-agent 3-dot menus.** Most of their entries name GUI-only concepts (open in new pane, open in new tab, focus the owning pane); the TUI has no panes. The one entry with real value, "view in Oz", already exists: `TuiCloudRunView` binds `Enter` to open the run URL. See §4 for how that binding is kept intact.
3. **Pinning.** Deliberate divergence; see §8.
4. Rendering conversations without a retained, navigable session — unchanged from CODE-1822 clause 4.
5. A second row, an expandable tree, or an overlay. The bar stays exactly one row.
6. Any change to orchestration behavior, depth policy, or the server-side depth budget. This is a presentation change only.
7. Surfacing a numeric remaining-depth budget anywhere. The client cannot know it; see §6.

## Behavior

### 1. Level scoping and the anchor
1. The bar renders exactly one level of the orchestration tree: the **anchor** conversation, followed by the anchor's **direct** children in canonical pill order.
2. The anchor is resolved as follows, in order:
   a. If the user explicitly descended into a conversation (§4) and the active conversation is that conversation or one of its descendants, the anchor is the descended-into conversation.
   b. Otherwise, if the active conversation has at least one navigable child, the anchor is the active conversation.
   c. Otherwise, the anchor is the active conversation's parent, so a leaf always shows its siblings.
   d. At the tree root, the anchor is the root. This reproduces today's behavior exactly.
3. Explicit descent state is discarded as soon as the active conversation leaves that subtree, so the anchor can never point somewhere the user cannot see.
4. The anchor occupies the bar's existing main-tab slot. Its label is the anchor's agent name, or `orchestrator` when the anchor is the tree root — so at depth 1 the label is unchanged.
5. The anchor tab gains a leading glyph showing `aggregated_orchestrator_status` for its subtree. A finished orchestrator with running children reads `●`, not blank. This closes the "no aggregated orchestrator status" gap without new logic.
6. Child ordering continues to use `child_conversations_in_pill_order`. The TUI must not maintain a separate ordering policy. CODE-1822 clauses 9 and 10 carry over unchanged, scoped to the level.
7. **Supersedes CODE-1822 clauses 3, 5, and 30-33** (bar contents, orchestrator label, orchestrator fixed at the leading edge).

### 2. Breadcrumbs
8. When the anchor is not the tree root, one **breadcrumb chip** renders per ancestor, to the left of the anchor and to the right of the `Agents:` label, in root-to-parent order.
9. A breadcrumb chip is a real, selectable, clickable tab: a `‹` leading marker plus the ancestor's name capped at **12** display cells (children keep 20). Selecting it switches to that conversation, which by rule 2c/2b re-anchors the bar one or more levels up. **This is the ascend mechanic; there is no separate ascend key.**
10. Breadcrumb chips never paginate and are never hidden by the child region's overflow. They degrade by shrinking (§5), never by disappearing, so ascent is always reachable.
11. At realistic depths this is at most two chips. The proposal does not cap the count; the degradation ladder handles the width, and depth is bounded by the server's depth budget.
12. `←` from the anchor moves to the nearest breadcrumb chip rather than wrapping to the last child; `←` from the first breadcrumb wraps to the last child of the level. Wrapping stays within the rendered row, so the row is always a closed cycle.

### 3. Group children
13. A child with at least one loaded descendant is a **group** child and renders a trailing badge `▸N`, where N is `LoadedSubtreeRollup::descendant_count` — the loaded-descendant count, so the badge never advertises nodes the rollup did not account for.
14. A group child's **leading glyph shows its rolled-up subtree status** (`orchestration_aware_conversation_status`), not its own status. A parent whose own turn finished while a grandchild still runs reads `●`. There is only one glyph slot per tab; the aggregate is the more useful occupant.
15. **A group child's position in the row is sorted by its own status, not the rollup.** A grandchild's lifecycle must never reorder the level the user is looking at. This is not negotiable; it is the cheapest available defense against churn.
16. A non-group child renders exactly as today: one glyph, one label, no badge.
17. The **anchor never carries a group badge**. Its children are the rest of the row; a count would be redundant.
18. Reading order within a tab is therefore `[status-or-identity glyph] [label] [▸N]`, which keeps the existing leading edge untouched and puts the new information where it does not compete with it.

```
● researcher ▸3      running, 3 loaded descendants
⟡ implementer        finished leaf, identity glyph
■ reviewer           blocked leaf
● crawler ▸2         itself finished, but 2 descendants still running
```

### 4. Navigation
19. `←` / `→` move **within the current level**, across `[breadcrumb chips…, anchor, children…]`, wrapping within that row. They never change the level except as a consequence of selecting a breadcrumb.
20. `Tab` / `Shift+Tab` keep today's **tree-wide** walk over `[root, all descendants in pill order]`. Landing on a conversation at another depth re-anchors the bar per rule 2. This preserves today's muscle memory and guarantees every agent is reachable in a bounded number of presses without the user having to understand the hierarchy.
21. `Enter` **descends** into the selected tab when it is a group child, making it the anchor. On a non-group tab it does nothing.
22. `Shift+←` / `Shift+→` select the first / last child **of the current level**. Semantics unchanged, scope narrowed. **Supersedes CODE-1822 clauses 22-24.**
23. `↓` / `Shift+↓` leave the bar, `Esc` returns to the tree root's session, and `Ctrl+C` kills the selected child. All unchanged.
24. Clicking a breadcrumb chip ascends. Clicking a tab selects it, as today; clicking an already-selected tab remains a no-op (CODE-1822 clause 32 is preserved). The `▸N` badge is its own hit target and descends when clicked, which is the mouse equivalent of `Enter`.
25. The focused footer becomes level-aware, extending the existing pattern in `render_orchestration_child_selected_tab_footer`:
    - default — `Tab or ← → to navigate  Shift + ← → to go to start/end  ↓ to send a message`
    - group child selected — `… Enter to open sub-agents  Ctrl+C to kill sub-agent`
    - drilled in — `… ← to go back`

#### Keymap conflict audit
Checked against every binding registered in `crates/warp_tui`. Within `<surface> & TuiOrchestrationTabBarFocused`, the bound keys today are `left`, `right`, `tab`, `shift-tab`, `shift-left`, `shift-right` (`orchestration_tab_bar.rs`), plus `down`, `shift-down`, `escape` and fixed `ctrl-c` (`terminal_session_view.rs:918-945`). Two findings:

- **`enter` is free on the terminal-session surface.** `tui:input:submit` is scoped to `id!("TuiInputView")`, and `focus_current_owner` calls `ctx.focus_self()` on the session view while the bar is focused, so the input is not in the keymap context. `ctrl-enter` and `ctrl-shift-enter` are distinct keystrokes.
- **`enter` is NOT free on the cloud-run surface.** `cloud_run_view.rs:78-85` binds `enter` to `tui:cloud_session:open_url` against the whole view context, including while the bar is focused. Implementing `Enter`-to-descend therefore requires narrowing that binding with `& !id!(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG)`. The Oz link stays reachable by leaving the bar. This is a required, named code change, not a footnote.

`shift-up` was considered for ascend and **rejected**: `cloud_run_view.rs:86-93` binds it view-wide to focus the tab bar, so it would collide on the cloud surface. Making ascend a consequence of breadcrumb selection (rule 9) avoids needing a new key on either surface, which is the main reason that design is preferred over an explicit ascend binding.

### 5. Narrow terminals
26. The bar remains a single row at every width and never writes outside it. **CODE-1822 clause 44 is superseded** by an explicit ladder.
27. Chrome is shed before content. The ladder, in drop order:

- **T0**, ≥ 96 cols — everything: `   Agents:   `, breadcrumb labels at 12, anchor label at 20, child labels at 20, `▸N` badges.
- **T1**, < 96 — breadcrumb label cap 12 → 8; child label cap 20 → 16.
- **T2**, < 84 — the `Agents:` leading collapses to two cells of padding (frees 11).
- **T3**, < 72 — breadcrumb chips collapse to marker-only (`‹`, no label), still selectable and clickable; anchor label cap → 8.
- **T4**, < 64 — the anchor collapses to its glyph alone; child label cap → 12; badge `▸N` → `▸`.
- **T5**, < 56 — the badge is dropped; child label cap → 8.
28. Never dropped at any width: the `|` divider, the anchor's glyph, at least one breadcrumb marker when the bar is drilled in, the selected child's glyph plus at least one label cell with `...`, and any applicable overflow arrow. If those do not fit, the child region pages down to a single tab — the existing behavior.
29. The reasoning is that at 60 columns a user wants to see their agents, not the word `Agents:`. Today's fixed prefix is 31 cells (`   Agents:   ` 13, `orchestrator` tab 14, divider 4), which leaves 29 cells for children at 60 columns — enough for exactly one child plus an arrow. T4 cuts the prefix to 9 and fits three.

### 6. The permission card
30. When `FeatureFlag::MultiLevelOrchestration` is enabled, the acceptance card adds one dim line directly under the agent identity line: `↳ These agents may start their own child agents`. Same text and the same gate as the GUI (`run_agents_card_view.rs:1545`), so the two front-ends make the approver the same promise.
31. The card does **not** surface a remaining depth budget. The budget is server-side and the client cannot cheaply know it — the GUI documents exactly this at `run_agents_card_view.rs:1541-1544`. A number the client guessed would be a lie, and a vague one would be noise.
32. There is no nested-approval presentation, because there is no nested approval: `run_agents.rs:446-451` auto-executes `run_agents` for every child conversation, so a card inside a child never renders. Building a "you are approving at depth 2" treatment would be dead UI.

```
■ Can I start additional agents for this task?
   Agents (3):
   ⊹ researcher  •  ⟡ implementer  •  ✶ reviewer
   ↳ These agents may start their own child agents

   Location: Local  •  Harness: Warp  •  Model: Default model

Enter to accept  Ctrl + E to edit Ctrl + C to reject
```

### 7. Discoverability
33. The `?` overlay's existing `Orchestration` section is extended, while the bar is available, to list the level keys alongside the entry key: `Shift+↑ navigate to agents`, `Enter open sub-agents`, `← back to parent`. The section stays gated on `orchestration_available`; advertising keys that currently do nothing is worse than saying nothing.
34. The `▸N` badge is itself the discoverability affordance for descent — it is the only thing in the row that signals "there is more underneath", and the footer names the key whenever a group tab is selected (clause 25).
35. A `/agents` slash command focuses the bar when a tree exists, and otherwise explains in one line that agent tabs appear once the agent starts child agents. This is the only entry point that works before a tree exists. **Needs sign-off** — it adds a command to a menu the requester may want to keep short.

### 8. Deliberate divergences from the GUI
36. **No pinning.** Pinning exists in the GUI to tame a wide, mouse-driven, unbounded pill row. The TUI row is keyboard-first, has `Shift+←`/`Shift+→` jump-to-edges, and now has level scoping, which bounds the row far more effectively than pinning would. Pinning would add persisted state plus a keybinding for marginal benefit. Existing pin state continues to affect ordering for GUI parity, exactly as CODE-1822 clause 18 already says.
37. **No per-agent menus, no hover cards.** See non-goals 1 and 2. Recorded here so the divergence is deliberate and traceable rather than an omission.
38. `specs/orch-pill-bar-web/PRODUCT.md` non-goal 39 rules nesting out of scope for the web viewer. That remains true and is not affected; the web viewer and the TUI are answering different questions.

### 9. Inter-agent message depth (secondary)
39. When a received message's sender is **not** a direct child of the current conversation, its collapsed header prefixes the sender with its parent: `● ⟡ researcher › crawler`. When the sender is a direct child, the header is unchanged. This is the only place outside the bar where depth is worth spending cells, and it is cheap. Lower priority than everything above; drop it if it does not fit the first cut.

## Mockups
Widths are exact; where a ruler is shown, each digit is one display cell.

### At rest — root anchored, depth 1, 100 cols
Identical to today apart from the `●` on `orchestrator` and the `▸3` on `researcher`.

```
1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890
   Agents:   ● orchestrator | ● researcher ▸3   ⟡ implementer   ■ reviewer
```

### Drilled into a child — anchor `researcher`, 100 cols
One breadcrumb chip, because the parent is the root. The anchor drops its `▸3` badge, since its children now fill the row.

```
   Agents:   ‹ orchestrator  ● researcher | ● crawler ▸2   ◊ indexer   ● ranker
```

### With a grandchild present — anchor `crawler` at depth 2, 100 cols
Two breadcrumb chips, root then parent, matching the GUI's `breadcrumb_ids` rule.

```
   Agents:   ‹ orchestrator  ‹ researcher  ● crawler | ● fetch-a   ⊛ fetch-b
```

### Tree-wide `Tab` landing on a grandchild
The bar re-anchors to the grandchild's parent so the user sees its siblings, not a flat list. This is the moment the current flat row is worst and the proposal earns the most.

```
before Tab (anchor = root)
   Agents:   ● orchestrator | ● researcher ▸3   ⟡ implementer   ■ reviewer

after Tab lands on `indexer`, a child of `researcher`
   Agents:   ‹ orchestrator  ● researcher | ● crawler ▸2   ◊ indexer   ● ranker
                                                          ^^^^^^^ selected
```

### 80 columns — T1/T2, drilled in
Breadcrumb label at 8, `Agents:` collapsed, child labels at 16.

```
12345678901234567890123456789012345678901234567890123456789012345678901234567890
  ‹ orche...  ● researcher | ● crawler ▸2   ◊ indexer   ● ranker
```

### 60 columns — T4, drilled in
Breadcrumbs are markers, the anchor is its glyph. The divider still separates level from anchor, so the structure survives.

```
123456789012345678901234567890123456789012345678901234567890
  ‹  ● | ● crawler ▸2   ◊ indexer   ● ranker
```

### 60 columns — T4, root anchored
Today this width shows exactly one child. It now shows three.

```
  ● | ● researcher ▸3   ⟡ implementer   ■ reviewer
```

### 60 columns with overflow — six siblings at the level
```
  ‹  ● | ● crawler ▸2   ◊ indexer   ● ranker   →
```

## Decisions that need the requester
Each of these has a recommendation above; these are the ones where the call is genuinely his.

1. **Drill-down over a flat row with depth glyphs.** Recommended: drill-down. A flat row cannot say *whose* child a tab is without prefixing labels (`researcher › crawler`), which at a 20-cell cap leaves almost nothing for the actual name, and the row grows without bound as depth grows. The cost is one keystroke to reach a grandchild deliberately — and `Tab` still reaches it in one press anyway.
2. **`Enter` to descend, and narrowing the cloud view's `enter` binding to make room.** The alternative is leaving `Enter` alone and offering only the `▸N` click target, which abandons keyboard users.
3. **Ascend by selecting a breadcrumb rather than by a dedicated key.** Recommended because both natural candidates (`shift-up`, `up`) either collide on the cloud surface or read ambiguously against `↓ = leave the bar`.
4. **Freezing the row order while the bar has keyboard focus.** Clause 15 already stops grandchildren from reordering a level. Same-level churn can still move a tab under the cursor. Freezing fixes it but means a finished agent does not sink until focus leaves the bar. Recommended, but it is a real trade and the requester should pick.
5. **The degradation tiers in clause 27**, in particular dropping the `Agents:` label at 84 columns and the anchor label at 64. These are judgement calls about what a narrow-terminal user values.
6. **`/agents` slash command** (clause 35).
7. **Whether clause 39** (depth in message headers) is in the first cut or deferred.

## Implementation sketch
Not a plan; enough to show the proposal is buildable in the cell-grid element library and to price it. A `TECH.md` alongside this file follows approval, not before it — writing one now would price a design nobody has agreed to.

**`orchestration_model.rs`** — the centre of gravity. `TuiOrchestrationSnapshot` gains `anchor_conversation_id`, `breadcrumbs: Vec<TuiOrchestrationBreadcrumb>`, and `navigation_order: Vec<AIConversationId>`. `children` narrows from `descendant_conversations_in_pill_order(root)` to `child_conversations_in_pill_order(anchor)`; `navigation_order` keeps the old flat `descendant_conversations_in_pill_order(root)` walk, which is what `Tab` consumes. `TuiOrchestrationChild` gains `subtree: Option<LoadedSubtreeRollup>` and `rollup_status`, both from existing helpers in `app/src/ai/blocklist/orchestration_topology.rs` (`loaded_subtree_rollup`, `orchestration_aware_conversation_status`, `aggregated_orchestrator_status`) — no new aggregation logic. The explicit-descent anchor is caller-owned state living next to `tab_bar_paging`, resolved with the same "explicit override, else derived default" shape as `TuiTabBarPagingState::resolve`.

**`tab_bar.rs`** — three additions to the generic component: `TuiTab::with_trailing_text` (for `▸N`, with its own `MouseStateHandle` so it can be its own click target), an optional per-tab label cap so breadcrumbs can use 12 while children use 20 (the component already validates label widths per tab via `TuiTabBarConfigError::LabelWidthTooSmall`, so per-tab widths fit its existing shape), and `leading_tabs: Vec<TuiTab>` rendered fixed, before `main_tab`, outside the paginating region. The degradation ladder is the one genuinely new mechanism: `fixed_prefix_width`/`minimum_row_width` already drive width-variant selection through `page_variant_transitions` and `TuiSizeConstraintSwitch`, so the tiers slot in as additional width-keyed variants of the chrome rather than a parallel system. All of it composes from `TuiFlex`, `TuiText`, `TuiContainer`, `TuiConstrainedBox`, and `TuiHoverable` — no new element type.

**`orchestration_tab_bar.rs`** — builds breadcrumb chips, the anchor tab with its aggregated glyph, and the `▸N` trailing badge; registers `tui:orchestration_tabs:descend` on `enter`; extends the three footer variants.

**`cloud_run_view.rs`** — narrow `tui:cloud_session:open_url` to `view_context & !id!(ORCHESTRATION_TAB_BAR_FOCUSED_FLAG)`.

**`terminal_session_view.rs`** — `switch_to_orchestration_conversation` currently hardcodes `source_conversation_id: snapshot.root_conversation_id` with the comment "the TUI tab bar is root-anchored (no drill-down), so the anchor and the tree root coincide". That stops being true; the telemetry should send the anchor id, which finally makes TUI and GUI pill-bar telemetry comparable. Two call sites, both already commented.

**`orchestration_block/render.rs`** — one gated `TuiText` line.

**`terminal_session_view/state.rs`** — extra `TuiShortcut` entries in the existing `Orchestration` section.

### Consequence for the committed test
`orchestration_model_tests.rs::restoring_parent_materializes_supported_descendant_sessions` asserts, via the `snapshot_child_ids` helper, that `snapshot.children` comes back as `[child_id, grandchild_id]` after restore. Under level scoping, the root anchor's `children` is `[child_id]` alone, so the assertion fails — and, more importantly, it would stop proving what it is about, which is that restoration materialized a session for **both** descendants.

The fix preserves the intent rather than the literal: point the helper at the new flat `navigation_order` field, which still contains `[child_id, grandchild_id]`, and add a second assertion that `snapshot(grandchild_id).children == [grandchild_id]` — proving the grandchild's level is anchored on its own parent. The test gets stronger, not weaker. The existing `TuiSessions` count assertions and the `focus_conversation_session(grandchild_id)` reachability check are untouched.

Elsewhere, `tab_bar_tests.rs` and `orchestration_tab_bar` snapshot tests need new cases for breadcrumbs, badges, and each degradation tier; those are render-to-lines tests per the `tui-testing` conventions, which is the right medium for asserting the exact column budgets in §5.

## Open risk
The proposal has not been exercised against a live multi-level tree in the TUI. The captures on CODE-1944 are genuine but do not show any orchestration surface: every prompt on the available test account returned `ErrorStatus(501, "")`, so no agent turn completed and `run_agents` never fired. The mockups here are derived from the rendering code's exact cell accounting (`fixed_prefix_width`, `natural_tab_width`, `tab_fixed_columns`, `DIVIDER_PADDING_*`, `secondary_gap_columns = 3`, `ORCHESTRATION_TAB_LABEL_MAX_COLUMNS = 20`) rather than from a photograph. They should be treated as accurate about widths and structure, and unverified about how they feel. First implementation step should be to stand up a two-level tree on an account that can complete a turn and check the ladder at 60, 80, and 100 columns before any of the polish lands.
