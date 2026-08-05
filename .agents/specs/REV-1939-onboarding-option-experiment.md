# Spec: Onboarding “Choose how to start” option-count experiment — Warp client (REV-1939)

This is the `warpdotdev/warp` half of a multi-repository feature. The sibling
`warpdotdev/warp-server` spec owns the experiment definition, user bucketing,
GraphQL enum values, eligibility, and traffic configuration. This client spec
owns consuming that assignment, rendering the correct onboarding arm, and
emitting arm-qualified client telemetry. The server behavior is an input
contract; no server internals are specified here.

Linear: <https://linear.app/warpdotdev/issue/REV-1939/onboarding-choose-how-to-start-ab-experiment-2-option-control-vs-3>
Originating thread: <https://warpdev.slack.com/archives/C0BDQDW8V5E/p1785884690170659>

All file references are pinned to Warp commit
`5c15a00751f7e57ab477deb00e44d3e90fc1da33` (base branch `master`). The exact
control copy is reconstructed from the parent of merge commit
`c7e3c4a032c55fb5e902991815108838396ccbdb` (PR #14605), rather than from memory.

== PRODUCT ==

*Summary:* Run a server-assigned A/B experiment on the account-first,
post-authentication `free_standard` “Choose how to start” slide. The control
restores the historical two-option screen; the experiment keeps the current
three-option screen when purchasable credit packs are available. The client
records the assigned arm on the complete onboarding monetization funnel so paid
completion can be compared without depending exclusively on an offline
assignment join.

*Key design choices:*
1. The control uses the exact pre-#14605 copy and behavior. The arms therefore
   primarily differ by option count and the current pack-aware plan card, rather
   than introducing a second copy experiment.
2. The client reads the two server experiment arms directly and represents
   `control`, `experiment`, and `unassigned` explicitly. It does not collapse the
   assignment into a boolean feature flag.
3. Control and unassigned users always receive the safe historical two-option
   layout. Experiment users receive three options only when credit packs are
   actually available; otherwise they receive the same historical two-option
   fallback.
4. Paid completion is the conversion signal. The assigned arm is attached to
   slide view, confirmed option action, upgrade start/completion, credit-purchase
   start/completion, and onboarding completion events. Click/start events remain
   diagnostic secondary signals.

*Behavior* (numbered, testable invariants from the user’s view):
1. This experiment affects only the account-first, post-authentication
   `OfferVariant::ChooseHowToStart` path for `free_standard` users. Paid users,
   `free_icp` users on `OfferVariant::HeadStart`, legacy onboarding, and every
   non-offer surface behave exactly as before.
2. A user assigned the control arm sees exactly two options even when pricing
   and purchasable credit packs are loaded:
   - Primary label: **“Use Warp with AI”**
   - Primary description: **“Warp Agent works locally or in the cloud with
     frontier and OSS models. Proactively fix terminal errors, implement
     changes, and ship verified code.”**
   - Secondary: the existing **“Set up AI later”** card and copy
   - No **“Buy AI credits”** card, credit-pack tiles, or pack-selection state is
     rendered.
3. Confirming the control primary sends the stable
   `use_warp_with_ai` onboarding action and follows the existing upgrade path,
   opening the URL produced by `AuthManager::upgrade_url()` (the `/upgrade`
   flow). Confirming “Set up AI later” follows the existing
   `free_standard_setup_later` completion path.
4. A user assigned the experiment arm who has one or more purchasable credit
   packs sees the current post-#14605 three-option layout, in order:
   **“Subscribe to a Warp plan”**, **“Buy AI credits”** with the current pack
   tiles, and **“Set up AI later.”** The current plan/add-on copy, selection,
   keyboard navigation, checkout, retry, and completion behavior are preserved.
5. In the experiment arm, confirming **“Subscribe to a Warp plan”** follows the
   same upgrade path as invariant #3. Confirming **“Buy AI credits”** starts the
   existing `purchase_addon_credits` flow with the selected denomination and
   team UID behavior unchanged. Onboarding completes only after the client
   observes synchronous purchase success or server-authoritative AI credit
   availability after browser checkout.
6. An experiment-assigned user with no available packs (pricing not loaded,
   purchase policy disabled, an empty server list, or any equivalent current
   `onboarding_credit_packs` outcome) sees the historical two-option layout and
   copy from invariant #2. The user remains telemetry-assigned to `experiment`;
   lack of packs must never be rewritten as a control assignment.
7. An unassigned user sees the historical two-option layout and copy from
   invariant #2. This is the default when neither arm is present at offer entry.
   The client snapshots the arm immediately before showing the post-auth offer;
   later server refreshes do not change the visible arm mid-exposure. This keeps
   the layout and every funnel event on one assignment.
8. If malformed server state contains both arms, the client fails closed to
   `unassigned` and the two-option layout. It never guesses an arm or exposes
   the pack purchase option from ambiguous state.
9. If the buy-credits option was selected and packs later become unavailable,
   selection falls back to the primary option as today’s `effective_choice`
   logic does when packs disappear. An already-started purchase is not
   cancelled; its existing success/failure handling continues.
10. Existing stable telemetry identifiers remain stable:
    `choose_how_to_start`, `use_warp_with_ai`, `buy_ai_credits`, and
    `set_up_later`. Relevant onboarding events add
    `experiment_arm: "control" | "experiment" | "unassigned"`; existing
    `flow_version`, `slide_name`/`source_slide`, `account_class`, action, and
    completion fields remain intact.
11. The client funnel is measurable as:
    - Exposure: `onboarding_slide_viewed` for `choose_how_to_start`
    - Confirmed option: `onboarding_action`
    - Plan path: `onboarding_upgrade_started` then
      `onboarding_upgrade_completed`
    - Pack path: new `onboarding_credit_purchase_started` then
      `onboarding_credit_purchase_completed`
    - Terminal outcome: `onboarding_completed`
    Every listed event emitted for this offer carries the same assignment that
    controlled the offer. Purchase events additionally carry the selected
    credit denomination. Paid completion (upgrade completed or credit purchase
    completed) is the client-side conversion; option and start events are
    secondary funnel signals.

== TECH ==

*Context:*
- `crates/onboarding/src/slides/offer_slide.rs:36-157` defines
  `OfferVariant::ChooseHowToStart`, its current plan/pack copy, stable action
  names, and `supports_credit_packs`. `OfferSlide::credit_packs`,
  `shows_credit_packs`, and `choices` at `:240-284` currently show packs whenever
  the variant supports them and the model has a non-empty pack list. Rendering
  at `:372-434` currently changes only the primary description based on
  `shows_credit_packs`; the primary label is always “Subscribe to a Warp plan.”
- The exact historical control at `c7e3c4a0^` used
  `OfferVariant::ChooseHowToStart => "Use Warp with AI"` and the description in
  product invariant #2. It had only `Primary` and `SetUpLater`.
- `crates/onboarding/src/model.rs:258-341` owns the post-auth offer variant and
  credit-pack/purchase state. Pack updates and purchase transitions live at
  `:500-607`; slide-view telemetry is emitted by `set_step` at `:1231-1321`.
  `crates/onboarding/src/agent_onboarding_view.rs:44-89` bridges these model
  events to the app and `:455-486` exposes the current post-auth offer entry
  point.
- `app/src/ai/onboarding.rs:49-72` builds eligible, premium-adjusted
  `onboarding_credit_packs`; an empty vector is the existing unavailable
  signal. `app/src/root_view.rs:2175-2269` seeds and refreshes those packs and
  bridges workspace/credit-availability events into onboarding.
- Upgrade start/completion and terminal onboarding completion are emitted in
  `app/src/root_view.rs:2411-2534` and `:2759-2805`. Credit purchase mutation and
  completion routing are handled at `:139-184` and `:2939-2962`.
- `app/src/server/experiments/mod.rs:23-137` defines client-visible
  `ServerExperiment` arms. `model.rs:14-89` caches and exposes the latest
  assignment set and emits `ExperimentsUpdated`; `convert.rs:10-136` owns
  persistence strings and GraphQL conversion. A direct arm query precedent is
  `runner_controls_enabled` in
  `app/src/ai/blocklist/inline_action/orchestration_controls.rs:69-80`.
- The client GraphQL enum is represented in
  `crates/graphql/src/api/experiment.rs:1-75` and mirrored in the checked-in
  schema snapshot at
  `crates/warp_graphql_schema/api/schema.graphql:1326-1397`.
- `crates/onboarding/src/telemetry.rs:24-237` defines onboarding events and
  payloads. `OnboardingAction`, upgrade events, and completion events do not
  currently carry an experiment assignment, and there are no dedicated client
  credit-purchase start/completion events.

*Design alternatives:*
- **Direct server-arm read vs. a feature-flag flip in
  `ServerExperiment::on_added_to`.** A boolean flag can represent “experiment
  enabled,” but cannot distinguish control from unassigned for telemetry, and
  `on_added_to` has no symmetric removal hook when the latest server set drops
  an arm. Directly querying `ServerExperiments` preserves all three states and
  follows the macOS-runner precedent.
  **Chosen: direct arm read; both new `on_added_to` match arms are explicit
  no-ops. No new `FeatureFlag` is added.**
- **Store the arm in onboarding state vs. query app singletons from the slide.**
  The reusable `onboarding` crate cannot depend on the app crate’s
  `ServerExperiments`, while slide-view/action/purchase telemetry is emitted
  inside that crate. **Chosen: define a small
  `ChooseHowToStartExperimentArm` value in `onboarding`, default it to
  `Unassigned`, and let `RootView` snapshot it from `ServerExperiments` at offer
  entry.**
  This keeps the crate boundary one-way and gives all client funnel events one
  consistent assignment source.
- **Suppress packs in `onboarding_credit_packs` vs. gate them in `OfferSlide`.**
  Suppressing data in the app helper conflates “not in the experiment” with
  “pricing unavailable,” makes assignment dependent on refresh ordering, and
  weakens tests. **Chosen: keep `onboarding_credit_packs` as the
  policy/pricing data source and make `OfferSlide::shows_credit_packs` require
  both the experiment arm and a non-empty list.** The model may retain loaded
  pack data while control/unassigned rendering hides it.
- **Historical copy vs. requester-recalled copy.** The recalled “Set up Warp AI”
  / “Purchase credits to try it out…” wording would change copy and option count
  simultaneously. **Chosen by requester: exact pre-#14605 copy**, yielding a
  cleaner option-count experiment.
- **Offline assignment join only vs. arm-qualified client funnel.** A server join
  can recover assignment, but makes event validation and funnel slicing more
  fragile. Attaching the arm only to option clicks would measure intent rather
  than paid conversion. **Chosen by requester: preserve the server join and
  additionally attach the arm to the full client-observed funnel, with dedicated
  purchase start/completion events.**

*Proposed changes:*
1. **Add and convert the client experiment arms.**
   - Add `OnboardingChooseHowToStartCreditsControl` and
     `OnboardingChooseHowToStartCreditsExperiment` to `ServerExperiment`.
   - Map them to the sibling server contract’s GraphQL values
     `ONBOARDING_CHOOSE_HOW_TO_START_CREDITS_CONTROL` and
     `ONBOARDING_CHOOSE_HOW_TO_START_CREDITS_EXPERIMENT` in the Cynic enum,
     checked-in schema snapshot, `TryFrom<Experiment>`, `Display`, and
     `from_string`.
   - Keep both `on_added_to` arms as no-ops because consumers query the
     assignment directly.
   - Add `ServerExperiments::choose_how_to_start_experiment_arm()` returning
     `Control`, `Experiment`, or `Unassigned`; neither/both maps to
     `Unassigned`.
2. **Carry assignment into onboarding state.**
   - Add public, copyable `ChooseHowToStartExperimentArm` in the onboarding
     crate with `Control`, `Experiment`, and default `Unassigned`, plus the
     stable telemetry strings from invariant #10.
   - Store it on `OnboardingStateModel`; expose a getter and an idempotent setter
     that notifies the view when the value changes.
   - Expose setter/getter forwarding on `AgentOnboardingView`.
   - In `RootView::resolve_account_first_post_auth`, immediately before
     `show_post_auth_offer`, resolve the latest `ServerExperiments` state and
     snapshot it onto the onboarding view. Do not mutate that arm after the
     offer is shown; a late assignment takes effect only on a future onboarding
     exposure.
3. **Gate the rendered option set, not pricing retrieval.**
   - Make `OfferSlide::credit_packs`/`shows_credit_packs` return/show packs only
     for `ChooseHowToStartExperimentArm::Experiment` and a non-empty eligible
     pack list.
   - Make the `ChooseHowToStart` primary label use “Subscribe to a Warp plan”
     only when packs are shown; otherwise use “Use Warp with AI.” Keep the
     existing `primary_description(shows_credit_packs)` behavior, which already
     selects the exact historical description when packs are not shown.
   - Keep `supports_credit_packs` as the variant capability check and keep
     `onboarding_credit_packs` policy, premium, denomination, and refresh logic
     unchanged.
4. **Attach the arm to existing onboarding events.**
   - Extend `SlideViewed`, `OnboardingAction`,
     `OnboardingUpgradeStarted`, `OnboardingUpgradeCompleted`, and
     `OnboardingCompleted` with an optional `experiment_arm`.
   - Populate it from `OnboardingStateModel`/`AgentOnboardingView` for the
     `ChooseHowToStart` offer funnel. Existing non-offer and `HeadStart` callers
     pass `None`, preserving their payloads.
   - Retain the assignment through `complete_account_first` long enough to add
     it to upgrade completion and terminal `onboarding_completed` before the
     root view transitions to the terminal.
5. **Add explicit credit-purchase funnel events.**
   - Add `OnboardingCreditPurchaseStarted` and
     `OnboardingCreditPurchaseCompleted` variants with event names
     `onboarding_credit_purchase_started` and
     `onboarding_credit_purchase_completed`.
   - Payloads contain `flow_version`, `source_slide`, `account_class`,
     `credits`, and `experiment_arm`.
   - Emit “started” only after a valid selected pack moves the model into
     `Purchasing`; emit “completed” only when
     `on_credit_purchase_completed` accepts an in-flight purchase after
     synchronous success or observed post-checkout credit availability.
     Rejected/abandoned checkout does not emit completion.
6. **Keep stable action and product paths.** Do not rename existing action,
   completion, or slide identifiers; do not modify the `/upgrade` page, checkout
   mutation, pricing calculations, team-UID selection, purchase-policy logic,
   or server assignment semantics.

*Open questions resolved:*
- **Control copy:** resolved by requester to exact pre-#14605 copy (“Use Warp
  with AI” plus the historical description), not the recalled alternate copy.
- **Client conversion:** resolved by requester to paid completion. Plan
  conversion is `onboarding_upgrade_completed`; pack conversion is
  `onboarding_credit_purchase_completed`. Views, confirmed actions, and starts
  are secondary funnel signals.
- **Conversion window and aggregate metric:** owned by the sibling server/data
  contract, not the client. This client emits exact event timestamps and arm
  values; it does not calculate session/7-day conversion locally.
- **Fallback:** unassigned and all packs-unavailable states use the historical
  two-option layout. An experiment assignment remains `experiment` in telemetry
  even when its UI falls back; the client never relabels it as control.
- **Feature flag:** none. Direct assignment is required to distinguish all
  states and avoid sticky flag state.
- **Cross-repo enum contract:** fixed to
  `ONBOARDING_CHOOSE_HOW_TO_START_CREDITS_CONTROL` and
  `ONBOARDING_CHOOSE_HOW_TO_START_CREDITS_EXPERIMENT`; the sibling server PR
  must expose these exact values before the client PR can compile against its
  refreshed schema.

*Risks / blast radius:*
- Experiment data can be absent at offer entry. Mitigation: resolve the latest
  state immediately before the offer is shown, then freeze it for that exposure;
  default/unassigned is the safe two-option layout.
- A feature-flag approach could leave stale global state across server refreshes.
  Mitigation: no flag; derive from the latest arm set and store the explicit
  arm on the view.
- Telemetry enum field additions touch multiple call sites and can accidentally
  add `null` keys to unrelated events. Mitigation: optional arm fields are
  inserted only when `Some`; payload snapshot tests pin unchanged non-offer
  payloads.
- The experiment arm may be valid while packs are temporarily unavailable.
  Mitigation: UI falls back safely while telemetry preserves the assigned arm;
  paid completion remains an intent-to-treat measurement.
- Hiding packs after a state refresh can leave `BuyCredits` selected.
  Mitigation: preserve the existing `effective_choice` fallback and test arm
  transitions.
- New purchase telemetry must not count abandoned checkout as conversion.
  Mitigation: emit completion only from the existing accepted
  `on_credit_purchase_completed` transition.

*Validation & verification criteria* (must ALL pass before merge):
1. **GraphQL and persistence conversion.** Unit tests cover both new arms through
   GraphQL `TryFrom`, `Display`, and `from_string` round trips, and the checked-in
   GraphQL schema contains exactly the two server-owned enum values. Unknown
   values remain ignored by the existing conversion macro. Checked by
   `cargo nextest run -p warp_graphql -p warp`.
2. **Arm resolution.** `ServerExperiments` tests prove:
   control only → `Control`; experiment only → `Experiment`; neither →
   `Unassigned`; both → `Unassigned`; a cached arm and a later
   `apply_latest_state` update return the latest value before offer entry.
   Checked by
   `cargo nextest run -p warp server::experiments`.
3. **Control and unassigned rendering.** Extend
   `crates/onboarding/src/slides/offer_slide_tests.rs` with tests that load
   purchasable packs but set `Control` or `Unassigned`, then assert the choices
   are exactly `[Primary, SetUpLater]`, the historical label/description match
   invariant #2 byte-for-byte, no credit tiles render, and keyboard navigation
   skips `BuyCredits`. These tests fail on the current unconditional pack
   behavior and pass after the change. Checked by
   `cargo nextest run -p onboarding`.
4. **Experiment rendering and fallback.** Offer-slide tests prove:
   experiment + packs → `[Primary, BuyCredits, SetUpLater]` with current
   three-option copy and pack tiles; experiment + empty packs → historical
   two-option copy; a transition from experiment/packs/buy-selected to control
   makes the effective selection `Primary`; `HeadStart` never shows packs in any
   arm. Checked by `cargo nextest run -p onboarding`.
5. **Actions retain existing behavior.** Tests confirm the control primary and
   experiment subscribe primary both emit `use_warp_with_ai` and request the
   existing upgrade path; experiment buy-credits requests the selected pack;
   setup-later completes as before; an in-flight purchase still prevents a
   duplicate request. Existing REV-1886 purchase tests remain green. Checked by
   `cargo nextest run -p onboarding -p warp`.
6. **Pricing/policy fallback.** Existing and extended app tests prove
   `onboarding_credit_packs` still returns empty for missing pricing or a
   disallowed purchase policy and returns premium-adjusted packs when eligible.
   Loaded packs are retained in onboarding state but hidden for
   control/unassigned. Teamless and team-scoped purchase forwarding tests remain
   green. Checked by `cargo nextest run -p warp`.
7. **Telemetry payloads.** Extend
   `crates/onboarding/src/telemetry_tests.rs` to assert:
   - `choose_how_to_start` slide view and each offer action carry the expected
     `experiment_arm`;
   - upgrade started/completed and terminal completion carry the same arm;
   - credit-purchase started/completed carry arm, source slide, account class,
     and selected credits;
   - rejected or merely checkout-required purchase does not emit completed;
   - `HeadStart` and non-offer payloads omit `experiment_arm` rather than emit
     `null`;
   - existing names, `flow_version`, account classification, action, and
     completion fields are unchanged.
   Checked by `cargo nextest run -p onboarding -p warp`.
8. **Assignment snapshot timing.** An app-level test proves the latest
   `ServerExperiments` state is copied immediately before
   `show_post_auth_offer`, so control/experiment are honored even when the view
   was constructed earlier. A second assertion proves an assignment update
   after the offer is shown does not mutate that exposure’s arm, layout, or
   telemetry value. Checked by `cargo nextest run -p warp`.
9. **No collateral product changes.** Tests and diff review confirm no behavior
   change to `HeadStart`, paid-user routing, legacy onboarding, the `/upgrade`
   page, credit pricing/premium calculations, checkout URL handling, or
   purchase team-UID selection.
10. **Repository checks pass.** Run `./script/format --check`,
    `cargo check -p onboarding -p warp_graphql -p warp`,
    `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings`,
    `cargo clippy -p warp --all-targets --tests -- -D warnings`, and
    `cargo nextest run -p onboarding -p warp_graphql -p warp`. PR CI is the
    full-suite backstop for this M-sized, bounded client change.
11. **Visual proof is attached (user-facing GUI change).** Using the Warp-client
    UI verification flow (`test-warp-ui`/computer use on an available runner),
    record a short video that exercises the real account-first
    `free_standard` path under forced assignments:
    - control with packs available shows exactly the historical two-option
      screen and primary opens the upgrade flow;
    - experiment with packs available shows the current three-option screen,
      pack tiles, and buy-credit selection;
    - experiment with packs unavailable and unassigned each show the safe
      historical two-option fallback.
    Validate the recording against invariants #2-#7 and attach it to both the
    Linear task record and final PR body. Media is not committed.

## Out of scope

- Server experiment definition, bucketing, eligibility, traffic percentage,
  configuration, or analytics-window computation (owned by the sibling
  `warpdotdev/warp-server` spec).
- Any change to the `/upgrade` page or its presentation of plans and add-on
  credits.
- Changes to credit-pack pricing, premiums, denominations, purchase policy,
  Stripe Checkout, or mutation arguments.
- A new client-only experiment layer, `ExperimentTriggered` event, or runtime
  `FeatureFlag`.
- Changes to Oz web FTUE (REV-1889), `OfferVariant::HeadStart`, paid-user
  onboarding, or legacy onboarding.
- New control copy, new card components, or a visual redesign. Both arms reuse
  the existing offer-slide component and existing copy from the selected code
  revisions.
