# Smooth Scrolling for Discrete Mouse-Wheel Input — Tech Spec

See [`PRODUCT.md`](./PRODUCT.md) for user-visible behavior.

Base commit:
[`5fb3144db9638c6c43371b566e1d0a89ae69236c`](https://github.com/warpdotdev/warp/tree/5fb3144db9638c6c43371b566e1d0a89ae69236c)

## Context
Warp preserves the distinction that winit provides between line-based and pixel-based wheel input.
The application multiplies line-based input, then each scroll consumer converts or applies that
delta immediately.

- [`crates/warpui/src/windowing/winit/event_loop/mod.rs:1254-1268`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui/src/windowing/winit/event_loop/mod.rs#L1254-L1268)
  maps `LineDelta` to `Event::ScrollWheel { precise: false }` and `PixelDelta` to
  `precise: true`.
- [`app/src/lib.rs:728-735`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/lib.rs#L728-L735)
  applies `ScrollSettings::mouse_scroll_multiplier` only to non-precise events.
- [`app/src/lib.rs:1885-1892`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/lib.rs#L1885-L1892)
  installs that transformation as the application event munger.
- [`app/src/settings/scroll.rs:4-14`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/settings/scroll.rs#L4-L14)
  defines the multiplier with default `3.0`.
- [`app/src/settings_view/features_page.rs:5021-5092`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/settings_view/features_page.rs#L5021-L5092)
  renders “Lines scrolled by mouse wheel interval” with range 1–20.

Generic GUI scrolling has two wrapper implementations and shared clipped state:

- [`crates/warpui_core/src/elements/gui/new_scrollable/mod.rs:34-55`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui_core/src/elements/gui/new_scrollable/mod.rs#L34-L55)
  defines the 40-pixel conversion for non-precise line input.
- [`crates/warpui_core/src/elements/gui/new_scrollable/mod.rs:731-835`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui_core/src/elements/gui/new_scrollable/mod.rs#L731-L835)
  converts and immediately applies deltas for single-axis and dual-axis `NewScrollable`.
- [`crates/warpui_core/src/elements/gui/new_scrollable/mod.rs:1344-1390`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui_core/src/elements/gui/new_scrollable/mod.rs#L1344-L1390)
  performs bounds and hit testing before the wrapper handles a wheel event.
- [`crates/warpui_core/src/elements/gui/scrollable.rs:328-349`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui_core/src/elements/gui/scrollable.rs#L328-L349)
  performs the equivalent immediate conversion in legacy `Scrollable`.
- [`crates/warpui_core/src/elements/gui/clipped_scrollable.rs:58-81`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui_core/src/elements/gui/clipped_scrollable.rs#L58-L81)
  stores one displayed scroll offset and exposes immediate `scroll_to` and `scroll_by`.

Terminal scrolling is a separate consumer:

- [`app/src/terminal/block_list_element.rs:1330-1395`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/terminal/block_list_element.rs#L1330-L1395)
  converts precise pixels to fractional lines, keeps non-precise input in line units, and decides
  whether a long-running-block event must become `AltMouseAction`.
- [`app/src/terminal/view.rs:9597-9607`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/terminal/view.rs#L9597-L9607)
  applies `TerminalAction::Scroll` through `ScrollPositionUpdate::AfterScrollEvent`.
- [`app/src/terminal/block_list_viewport.rs:1293-1328`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/terminal/block_list_viewport.rs#L1293-L1328)
  clamps fractional line positions and preserves follow-bottom and long-running-block state.
- [`app/src/terminal/alt_screen/alt_screen_element.rs:442-476`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/app/src/terminal/alt_screen/alt_screen_element.rs#L442-L476)
  accumulates alternate-screen wheel input for PTY behavior.

Warp already has a time-based animation driver for touch momentum:

- [`crates/warpui/src/windowing/winit/event_loop/mod.rs:63-78`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui/src/windowing/winit/event_loop/mod.rs#L63-L78)
  defines an 8-millisecond cadence and frame-rate-independent decay.
- [`crates/warpui/src/windowing/winit/event_loop/mod.rs:796-835`](https://github.com/warpdotdev/warp/blob/5fb3144db9638c6c43371b566e1d0a89ae69236c/crates/warpui/src/windowing/winit/event_loop/mod.rs#L796-L835)
  synthesizes precise wheel events for touch momentum.

The timer pattern is reusable. The momentum algorithm is not. Smooth wheel scrolling must own an
exact target and must not synthesize PTY-bound wheel input.

## Decisions
### Use a target tween, not velocity decay
Options:
- Decay a velocity, like touch momentum. This composes naturally, but it adds inertial travel and
  makes the final position depend on timing.
- Tween to an exact target. This preserves existing scroll distance and can stop without overshoot.

Use a target tween. Each input contributes a fixed destination. Cubic ease-out provides fast initial
response without adding momentum.

### Compose additive contributions, not animation restarts
Options:
- Restart one 120-millisecond tween after every notch. This makes rapid input feel delayed because
  progress repeatedly returns to the start of the easing curve.
- Add one 120-millisecond contribution per input batch. Existing contributions keep their progress,
  and the target is the sum of their endpoints.

Use additive contributions. Coalesce contributions created during the same animation frame. Keep
the active contribution list bounded by merging contributions that have the same direction and
equivalent start time.

For normalized time `t` in `[0, 1]`, use:

```rust
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}
```

At each frame, emit only the difference between the contribution's new eased amount and the amount
already emitted. At `t == 1`, emit any floating-point remainder and remove the contribution.

### Keep animation ownership at the scroll consumer
Options:
- Rewrite all non-precise wheel events in the winit event loop. This is simple, but it also rewrites
  events intended for terminal mouse reporting. The closed prototype
  [#13021](https://github.com/warpdotdev/warp/pull/13021) used this direction.
- Let each scroll consumer opt in after normal hit testing and PTY-routing decisions.

Keep one `SmoothScrollController` with each participating scroll state. Generic WarpUI wrappers own
the Phase 1 controllers. `TerminalView` owns the Phase 2 controller. The winit layer may drive frame
timing, but it must not own or reinterpret the target.

### Use one rollout flag
Use `FeatureFlag::SmoothScrolling` for both phases. Phase 1 can ship because Phase 2 code is not part
of the first implementation. Phase 2 reuses the flag when it lands. Do not add a permanent setting
or a second per-phase flag.

## Proposed changes
### Shared animation controller
Add a small, deterministic controller under `crates/warpui_core/src/smooth_scroll.rs`. Keep it
outside the GUI element module because Phase 2 reuses it from `TerminalView`.

The controller stores logical offsets as `Vector2F`/`f32`. The consumer converts those values to
`Pixels` or `Lines` at its existing boundary. The controller owns:
- The displayed offset for each participating axis.
- The clamped target offset for each axis.
- Active additive contributions with monotonic start times.
- The amount already emitted by each contribution.
- An animation generation token used to ignore stale frame callbacks after cancellation.

The controller API must support:
- `add_discrete_delta`: add a same-direction contribution to the current target.
- `reverse_with_delta`: materialize the current displayed position, clear old contributions, and
  start the opposite direction.
- `advance(now)`: return the incremental delta for this frame and whether another frame is needed.
- `cancel`: clear contributions and make target equal displayed position.
- `set_position_immediately`: cancel, then update displayed and target positions together.
- Independent horizontal and vertical state.

Inject `now` into controller methods. Do not read wall-clock time inside deterministic controller
tests. Use `instant::Instant` at the integration boundary.

Clamp the target before adding a contribution. Bounds checks and nested propagation must use the
target position, not only the lagging displayed position. This prevents an inner scrollable from
accepting input that belongs to its parent while an animation is still catching up.

### Frame driving
Add a view-scoped smooth-scroll frame request to `EventContext` and carry it through
`presenter::DispatchResult` and `windowing::EventDispatchResult`. A request contains:
- The requesting view ID from the current event-context stack.
- A process-unique controller ID.
- The controller generation.
- The next requested monotonic deadline.

When the deadline arrives, dispatch an internal `SmoothScrollFrame` event to the requesting view,
with the controller ID and generation. Containers forward this event without hit testing. Only the
matching persistent controller handles it. The handler calls `advance(Instant::now())`, applies the
incremental delta through the existing consumer API, and requests the next frame only when work
remains.

Follow the existing touch-momentum timer pattern:
- Use a monotonic, time-based calculation.
- Request frames at an 8-millisecond cadence so 120 Hz displays are not capped to 60 Hz.
- Coalesce redundant frame requests per window.
- Stop scheduling immediately when no participating controller is active.
- Do not use `AnimationClock` or `KeyframeTimeline`; those types select discrete keyframes and do not
  update a continuous scroll offset.
- Do not wrap every scrollable in a permanently live `LiveElement`.

Route a frame by requesting view, controller ID, and stable animation generation, not by the
cursor's current position. If the view or controller no longer exists, drop the request. An element
that was rebuilt without the same persistent handle or was cancelled must ignore the stale frame.

The frame callback applies the incremental delta through the consumer's existing scroll method.
This preserves view notification, selection anchoring, manual child state, and scrollbar updates.

### Feature flag and event eligibility
Add the standard flag plumbing:
- `app/Cargo.toml`: add `smooth_scrolling = []` under `[features]` and include it in `default`.
- `crates/warp_features/src/lib.rs`: add `FeatureFlag::SmoothScrolling`.
- `app/src/features.rs`: add the cfg-gated compiled-in registration.
- `crates/warp_features/src/lib.rs`: add `SmoothScrolling` to `RUNTIME_FEATURE_FLAGS` so local and
  dev builds can turn it off from the existing runtime feature menu.

Do not add the flag to `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, or `RELEASE_FLAGS`; the default Cargo feature
already enables it for compiled app targets.

Extend scroll-event metadata so the application event munger can mark a non-precise event as
animation-eligible after checking `FeatureFlag::SmoothScrolling.is_enabled()`. Keep precision and
animation eligibility as separate fields:
- `precise` continues to describe the physical input.
- Animation eligibility describes whether a participating consumer may tween it.
- Synthetic precise touch-momentum events remain ineligible.

Apply `apply_scroll_multiplier` before a consumer creates an animation target. Animation frames must
carry already-converted incremental pixels or lines and must bypass the multiplier.

Turning the flag off in local or dev takes effect for new input and cancels active controllers.
Turning it off for a shipped release requires removing the Cargo feature from the compiled default
and shipping a new build. No remote kill switch is introduced.

### Phase 1: generic WarpUI scrollables
Update both shared wrapper paths:
- `crates/warpui_core/src/elements/gui/new_scrollable/mod.rs`
- `crates/warpui_core/src/elements/gui/new_scrollable/single_axis_config.rs`
- `crates/warpui_core/src/elements/gui/new_scrollable/dual_axis_config.rs`
- `crates/warpui_core/src/elements/gui/scrollable.rs`
- `crates/warpui_core/src/elements/gui/clipped_scrollable.rs`

Store the controller in the existing persistent scroll handles, not in the ephemeral wrapper element:
- Add per-axis smooth state to `ScrollState` for manually managed wrapper axes.
- Add smooth state beside `scroll_start_px` in `ClippedScrollData`.
- Dual-axis clipped state keeps one controller channel per axis.

For an eligible non-precise event:
1. Keep the current multiplier and 40-pixel line conversion.
2. Project or remap the axes with the current logic.
3. Read current bounds and the controller target.
4. Propagate the event when the target cannot move in that direction.
5. Otherwise, clamp and add the delta to the controller.
6. Request animation frames and return handled.

For an ineligible event, use the existing immediate path.

Cancellation:
- A precise wheel event cancels the controller before applying its delta.
- Scrollbar mouse-down, gutter click, and drag cancel before their existing immediate operation.
- `ClippedScrollStateHandle::scroll_to`, `scroll_by`, and `scroll_to_position` cancel by default.
- Add an internal frame-only setter that updates displayed position without cancelling its own
  generation.
- For manually managed children, compare the child's reported `ScrollData.scroll_start` with the
  controller's last emitted position. A mismatch means an external operation moved the child;
  cancel before applying another frame.

The terminal block list and alternate screen are manual children of some shared wrappers. Preserve
their existing `axis_should_handle_scroll_wheel` routing so Phase 1 does not capture terminal-owned
vertical wheel events.

### Phase 2: normal terminal scrollback
Add a `SmoothScrollController` field to `TerminalView` using `Lines` as its logical unit. Reuse the
shared easing and lifecycle logic; do not convert terminal animation state to pixels.

In `BlockListElement::scroll_internal`, preserve routing order:
1. Reject out-of-bounds and disabled scrolling as today.
2. Convert precise pixels to fractional lines as today.
3. Determine whether a long-running-block event must be forwarded as `AltMouseAction`.
4. Forward PTY-bound input immediately and return. Do not touch the controller.
5. For normal block-list scrolling, dispatch the input source and delta to `TerminalView`.

In `TerminalView`:
- Precise input cancels the controller and uses `ScrollPositionUpdate::AfterScrollEvent` immediately.
- Eligible non-precise input updates the controller target.
- Each frame applies an incremental fractional `Lines` delta through the existing
  `scroll_position_for_delta` path.
- Reaching the bottom preserves `FollowsBottomOfMostRecentBlock`.
- Every non-animation `ScrollPositionUpdate` that directly changes position cancels active
  contributions. This includes page/home/end, block and find navigation, resize correction,
  command-driven follow-bottom, clear, and rich-block autoscroll.

Do not change `AltScreenElement::on_scroll`, `TerminalView::alt_scroll`,
`alt_screen_scroll_to_pty_bytes`, `TerminalAction::AltMouseAction`, or mouse protocol encoding except
for any exhaustive match updates required by new internal event metadata.

Phase 2 must be a follow-up implementation after Phase 1. It reuses the controller and frame driver
but does not block Phase 1 release.

## Testing and validation
### Automated tests
Add deterministic controller tests in a separate `smooth_scroll_tests.rs`:
- `ease_out_cubic_reaches_exact_target_without_overshoot`
- `same_direction_inputs_compose_without_restarting_existing_progress`
- `opposing_input_discards_unrendered_remainder`
- `late_frame_emits_exact_remaining_distance`
- `cancel_ignores_stale_generation`
- `horizontal_and_vertical_contributions_advance_independently`

Extend `new_scrollable/scrollable_tests.rs`, legacy `scrollable_tests.rs`, and
`clipped_scrollable_tests.rs`:
- One notch produces intermediate positions and the current final distance.
- The multiplier and 40-pixel conversion occur once.
- Feature-off behavior remains immediate.
- Precise input cancels and remains immediate.
- Scrollbar drag and programmatic scroll cancel.
- Same-direction and opposing input follow `PRODUCT.md` behaviors 5 and 6.
- Target-bound checks preserve nested propagation.
- Dual-axis input animates axes independently.
- A stale frame cannot move a rebuilt or cancelled scrollable.

Add Phase 2 terminal tests:
- Normal non-precise block-list input reaches the same final fractional-line position.
- Precise input cancels and applies immediately.
- Page/home/end, jump-to-bottom, find navigation, and follow-bottom updates cancel.
- Bottom clamping preserves `FollowsBottomOfMostRecentBlock`.
- Alternate-screen wheel input produces the same PTY bytes with the flag on and off.
- Mouse-reporting and long-running-block `AltMouseAction` receive exactly one unchanged action per
  source wheel event.
- No animation frame is emitted to a PTY.

Run focused tests while implementing:

```sh
cargo nextest run -p warpui_core scrollable
cargo nextest run -p warp terminal::block_list
```

Before each implementation PR is pushed, run:

```sh
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
./script/presubmit
```

### Human verification
For Phase 1, use a physical clicky wheel or a test path that injects `precise: false` input:
- Record a video of a long Settings page, a nested-scrollable surface, and a dual-axis surface.
- Show single notches, rapid same-direction notches, immediate reversal, trackpad interruption,
  scrollbar drag, keyboard scrolling, and boundary propagation.
- Repeat with the runtime flag disabled and confirm immediate movement.

For Phase 2:
- Record a video of long normal terminal scrollback with the same input sequences.
- Run `less` and Vim with mouse reporting where supported. Confirm that wheel behavior is immediate
  and command interaction is unchanged.
- Exercise a long-running block that forwards wheel input to the PTY.
- Exercise alternate-screen scrolling and a shared-session reader alternate screen.

Attach the videos to the applicable implementation PR descriptions. A screenshot alone does not
prove animation timing or interruption behavior.

### Cross-platform verification
The winit line-versus-pixel distinction and desktop event loops make this change OS-sensitive.
After local tests pass, use the `cross-platform-cloud-verification` workflow:
- Discover current runners at verification time.
- Select one representative macOS, Linux, and Windows runner when available.
- Prioritize Windows because GitHub issue #6169 was reported on Windows.
- Use the exact implementation branch and commit.
- Do not add an architecture matrix without architecture-specific evidence.
- Report any unavailable relevant platform as unverified.

On each platform, confirm:
- A line-delta event is smoothed.
- A pixel-delta event remains immediate.
- Final distance matches the multiplier.
- The animation stops scheduling frames after completion.
- PTY exclusions pass in Phase 2.

## Risks and mitigations
- **Redraw and GPU cost:** A short animation creates continuous frames, including for expensive
  clipped scrollables and large terminal scrollback. Schedule frames only while active, coalesce
  requests, and use time-based progress so missed frames do not create extra work.
- **Nested scrollables:** The displayed offset lags the target. Use the target for boundary
  acceptance so the inner child does not consume distance that belongs to its parent.
- **Manual scroll state:** Some wrappers delegate movement to a child. Apply frame deltas through
  the existing child API and detect external position changes before emitting another frame.
- **Dual-axis state:** Independent axes can finish at different times. Keep per-axis targets while
  sharing one frame request.
- **PTY regression:** Global input rewriting would corrupt mouse-reporting behavior. Opt in only
  after the terminal has decided that the event is ordinary scrollback.
- **Animation after direct navigation:** Any immediate position setter must cancel or change the
  generation before stale frames can run.
- **Suspend or delayed frames:** Use monotonic time, clamp normalized progress, and emit the exact
  remainder when the app resumes.
- **User preference:** Some users prefer immediate notches. The rollout flag is the temporary
  opt-out. A permanent preference and reduced-motion behavior require a later product decision.
- **Platform differences:** Verify actual line and pixel input on each desktop OS. Do not tune
  platform-specific curves unless evidence shows the shared curve is unsuitable.

## Parallelization
Implementation is sequential across phases:
- **Phase 1:** Reuse this spec PR after approval. Own shared controller, frame driving, flag wiring,
  and generic WarpUI scrollables on `factory/smooth-scrolling-spec`.
- **Phase 2:** Start after the Phase 1 controller contract is merged. Use a follow-up branch
  `factory/smooth-scrolling-terminal` and a separate PR so terminal work cannot block Phase 1.

Within each phase, implementation and core unit-test edits touch the same state and should remain one
workstream. After local validation passes, platform verification can fan out to independent remote
agents, one current runner per relevant operating system.

## Follow-ups
- Decide whether to add a user-facing smooth-scrolling preference.
- Add system reduced-motion integration if Warp introduces a shared reduced-motion policy.
- Remove `SmoothScrolling` and its immediate fallback after rollout is stable.
