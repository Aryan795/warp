# Smooth Scrolling for Discrete Mouse-Wheel Input — Tech Spec

See [`PRODUCT.md`](./PRODUCT.md) for user-visible behavior, including the amendment that
supersedes the flat 120ms ease-out cubic described in this Tech Spec's original "Decisions" and
"Proposed changes" sections below. See "Amendment: Chrome-style easing and duration" further down
for what actually shipped for Phase 1, and why.

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
**Superseded by the amendment below.** This section is kept for history; Phase 1 did not ship
with additive contributions.

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

### Amendment: retarget a single running animation, preserving velocity
Hands-on feedback after the Phase 1 implementation was that the additive-contributions model
above, combined with a flat 120ms ease-out-only cubic, did not feel smooth enough. The requester
asked for Chromium's `cc::ScrollOffsetAnimationCurve` model instead. This section replaces
"Compose additive contributions" and its ease-out snippet above.

Options considered (costed for the requester before they decided):
1. Duration change alone (flat longer value, or Chrome-style inverse-delta ramping).
2. Velocity-preserving retarget of a single running curve, replacing stacked contributions.
3. A full mass-spring-damper model.

The requester chose to adopt (1) and (2) together, matching Chromium's actual behavior, rather
than (3) (Chromium itself does not use a spring model for wheel scrolling, despite that being a
common assumption).

Use one running segment per axis instead of a list of independent contributions:
- **Easing**: cubic bezier ease-in-out, control points `(0.42, 0)` and `(0.58, 1)` -- the same
  curve as CSS's `ease-in-out` keyword -- evaluated by solving `x(t) = elapsed_fraction` for the
  bezier parameter `t` (Newton-Raphson, falling back to bisection), then returning `y(t)`, the
  same technique browsers use for `cubic-bezier()` timing functions.
- **Duration**: inversely proportional to the notch's delta magnitude
  (`DurationBehavior::kInverseDelta`). Chromium's published reference points are
  `kInverseDeltaMaxDuration` = 200ms at a 120px delta and `kInverseDeltaMinDuration` = 100ms at
  480px, clamped at both ends; Chromium's exact interpolation between them is not available to
  us, so fit a simple `A / delta + B` hyperbola through those two points instead.
- **Retarget**: on a same-direction notch arriving mid-flight, reshape the curve so its starting
  slope (in normalized time/progress space) matches the outgoing segment's velocity at that
  instant, rather than starting a fresh, independent, zero-velocity contribution. Implement this
  by holding the bezier's first control point's `x`-coordinate fixed and solving for its
  `y`-coordinate from the desired starting slope (`y1 = slope * x1`, clamped to avoid visible
  overshoot), keeping the second control point fixed so every curve still eases out to a stop at
  its target. This is the same curve-reshaping Chromium's `EaseInOutWithInitialSlope` describes
  doing, though its exact reshaping formula is not published, so this is our own derivation of an
  equivalent construction.
- **Velocity-based duration bound**: cap a retarget's duration so its reshaped starting slope
  cannot exceed a safe threshold when the controller is already moving fast toward a small
  remaining distance -- Chromium's `VelocityBasedDurationBound`, which exists specifically to
  stop a rubber-banding overshoot in that scenario.
- Opposite-direction input keeps the existing behavior unchanged: cancel at the currently
  displayed position (zero velocity), then ease in fresh toward the new target.

Every other approved behavior (exact landing on target, cancellation semantics, unchanged
multiplier and 40px-per-line conversion, independent axes) is unaffected.

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
**Implemented with one running segment per axis, per the amendment above, not a list of additive
contributions.** Added a small, deterministic controller under
`crates/warpui_core/src/smooth_scroll.rs`. Kept it outside the GUI element module because Phase 2
will reuse it from `TerminalView`.

The controller (`SmoothScrollController`) stores logical offsets as `f32`; the consumer creates
one instance per axis and converts to `Pixels` or `Lines` at its existing boundary. It owns:
- `committed`: the settled position, used whenever there's no active segment.
- An optional single `Segment { start, start_position, start_velocity, target, duration }`: the
  one in-flight motion, if any.

The controller API, as implemented:
- `add_delta(delta, now)`: retargets the running segment (same direction) or cancels and starts a
  fresh one (opposite direction, or starting from rest). See the amendment above.
- `displayed_position(now)`: the position that should currently be painted; settles a completed
  segment into `committed` as a side effect.
- `target()`: the exact position the controller is animating toward, ignoring progress. Used for
  bounds/propagation decisions instead of `displayed_position`.
- `is_animating(now)`: whether a segment is still easing in; also settles a completed segment.
- `cancel(now)`: settles at the currently displayed position and clears the active segment.
- `set_position_immediately(position)`: jumps directly to `position`, clearing any active segment.

Every method that depends on "now" takes an explicit `Instant` rather than reading the wall clock,
keeping the controller a pure, deterministic function of injected time -- this part of the
original proposal was implemented as designed.

Bounds and nested-propagation decisions use `target()`, not `displayed_position()`, so an inner
scrollable doesn't accept wheel input that belongs to its parent while its own animation is still
catching up -- also implemented as originally proposed.

### Frame driving
**Implemented differently from what this section originally proposed.** The
`EventContext`/`presenter::DispatchResult`/`windowing::EventDispatchResult` plumbing described
below was not built. Phase 1 instead reuses the existing `PaintContext::repaint_after` self
-scheduling mechanism already used by `LiveElement` and `ShimmeringTextElement`: every paint of an
animating axis calls `ctx.repaint_after(SMOOTH_SCROLL_FRAME_INTERVAL)`, which re-arms a repaint
timer for as long as the controller reports `is_animating()`. This produces the same visible
behavior (an axis keeps repainting at the target cadence until its animation settles) with far
less new plumbing, since the controller is a pure function of injected time and needs no
per-frame "emit" bookkeeping, generation token, or new event type to route.

Measured cadence: a dedicated test
(`smooth_scroll_animation_drives_many_distinct_repaints_over_its_duration`) drives a real
wheel-triggered animation through this self-scheduling chain (the real async timer, not a mock)
and counts distinct paints; it observed on the order of a dozen or more distinct paints over one
animation's duration, consistent with the 8ms request interval not itself throttling below what
a real display could provide. Actual on-screen cadence is bounded by the platform's vsync/redraw
cadence, the same as every other continuous animation already in the app.

The original proposal below is kept for context on why frame driving needed to exist at all, but
its specific mechanism (a routable `SmoothScrollFrame` event with a controller ID and generation)
is not what shipped:

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
#### Amendment: cancellation-surface audit, and the shape that actually shipped
Before implementation, the ~20 `ScrollPositionUpdate` variants and the call sites that reach them
were audited to determine whether cancellation collapses into a single guard or needs bespoke
handling per call site -- Phase 1's manual/legacy-wrapper surface had turned out materially larger
than its first estimate, and this was the equivalent risk for Phase 2.

Finding: `TerminalView::update_scroll_position_locking` is the **sole** call site of
`ScrollState::update` (grep-verified: every one of the ~30 call sites across `view.rs` and related
files that changes scroll position -- page/home/end, jump-to-block, jump-to-exchange, filter
apply/clear, resize, clear, command-execution-started, agent-view entry/exit, and every other
`ScrollPositionUpdate` variant -- routes through this one function). This makes cancellation a
single, universal guard rather than an M-vs-L-defining per-site concern: cancel any in-flight
animation on every update **except** `AfterScrollEvent`, placed once inside
`update_scroll_position_locking`. The one call site that needs bespoke handling is the wheel-input
entry point itself (`TerminalView::scroll`), because it must distinguish an `AfterScrollEvent` that
is its own animation-frame increment (must not cancel) from one that is an immediate precise/
flag-off scroll (must explicitly cancel first, since the universal guard exempts the variant).
This audit found the cancellation surface materially cleaner than Phase 1's, not materially worse.

The rest of this section describes the actual shipped shape, which differs from the original
proposal below in one respect: rather than the controller emitting a full delta that
`TerminalView` applies as one incremental `Lines` value per repaint, the controller output is
applied via the same lazy, per-event-dispatch pattern already proven for Phase 1's `Manual`-axis
scrollables (`ScrollState`/`take_smooth_scroll_increment` in `crates/warpui_core/src/elements/gui/
scrollable.rs`), since `TerminalView::scroll` (which owns the only code path that can call
`update_scroll_position_locking`) requires a `ViewContext`, unavailable during `BlockListElement`'s
paint. Concretely:

- `SmoothScrollHandle` (`app/src/terminal/block_list_viewport.rs`) wraps a
  `SmoothScrollController` behind an `Arc<Mutex<_>>`, tracking only the relative, unapplied
  remainder of an animation -- not an absolute position, unlike `ClippedScrollStateHandle`.
  `TerminalView` owns the persistent handle; `TerminalViewRenderContext` and `BlockListElement`
  each hold a clone, mirroring how `horizontal_clipped_scroll_state` is already threaded through.
- `TerminalView::scroll(delta, precise, ctx)`: for precise input or the flag off, cancels the
  handle and applies `AfterScrollEvent` immediately, exactly as before. For eligible non-precise
  input, calls `SmoothScrollHandle::add_delta` and returns without touching `scroll_position`.
- `BlockListElement::paint` requests `ctx.repaint_after(SMOOTH_SCROLL_FRAME_INTERVAL)` while
  `is_animating()`, identically to Phase 1's `Manual`/`Clipped` axes.
- `BlockListElement::dispatch_event` unconditionally dispatches
  `TerminalAction::AdvanceSmoothScroll` on every event it receives (mirroring
  `Scrollable::dispatch_event`'s unconditional `advance_smooth_scroll` call exactly -- gating this
  on `is_animating()` first would race with a segment completing between the gate check and the
  actual increment-taking, silently dropping the final increment and landing short of the exact
  target).
- `TerminalView::advance_smooth_scroll` (the `AdvanceSmoothScroll` handler) takes the pending
  increment and, if non-zero, applies it via `update_scroll_position_locking(AfterScrollEvent{..})`
  -- the same existing path a direct scroll uses, so `scroll_position_for_delta`'s clamping and
  `FollowsBottomOfMostRecentBlock` transition logic runs unmodified, against whatever the block
  list's current state is that frame. This is what makes content arriving mid-animation a
  non-issue: each increment is small and re-resolved against current state, not a captured
  absolute target.

In `BlockListElement::scroll_internal`, the routing order is preserved exactly as originally
proposed (reject-out-of-bounds, precise-to-lines conversion, long-running-block
`AltMouseAction` decision, PTY-bound early return, then dispatch to `TerminalView`) -- confirmed
structurally clean: alt-screen input never reaches `BlockListElement` at all (a separate element
entirely, swapped in by `TerminalView::render`), and the long-running-block forwarding decision is
a single branch, so the animation cannot leak into either path. `TerminalAction::Scroll` gained a
`precise: bool` field (threaded through every construction site, including scrollbar-drag and
keyboard-single-line-scroll call sites, which pass `precise: true` since they are not real wheel
notches) since the action previously discarded that information before it reached `TerminalView`.

One pre-existing bug this exposed and fixed: `scroll_position_for_delta`'s `fix_to_bottom` check
used a raw `new_top >= max_scroll_top` comparison. An animated scroll composes its final position
from the last of several small increments rather than one one-shot delta; in one observed case a
large-overshoot animation settled a hair short of `max_scroll_top` (floating-point precision, not a
logic error), which the raw comparison rejected -- leaving the view in `FixedAtPosition` right at
the boundary instead of the `FollowsBottomOfMostRecentBlock` sticky-bottom mode an equivalent
immediate scroll would reach. Changed to the codebase's existing `heights_approx_gte` tolerance
(already used for every other boundary comparison in this file).

Do not change `AltScreenElement::on_scroll`, `TerminalView::alt_scroll`,
`alt_screen_scroll_to_pty_bytes`, `TerminalAction::AltMouseAction`, or mouse protocol encoding --
confirmed unchanged.

Phase 2 shipped as a follow-up PR after Phase 1, stacked on the Phase 1 branch so its diff shows
only the terminal work; it retargets to `master` once Phase 1 merges.

## Testing and validation
### Automated tests
Deterministic controller tests live in `smooth_scroll_tests.rs`, pinning the model described in
the amendment above (this list supersedes the original one, which named tests for the additive
-contributions model that was not shipped):
- `ease_in_out_reaches_exact_target_without_overshoot`
- `fresh_segment_eases_in_from_zero_velocity`
- `opposing_input_discards_unrendered_remainder_and_reverses_immediately`
- `same_direction_retarget_lands_exactly_on_the_combined_target`
- `same_direction_retarget_preserves_velocity_across_the_seam`
- `cancel_settles_at_displayed_position_and_stops_animation`
- `set_position_immediately_overrides_in_flight_animation`
- `zero_delta_is_a_no_op`
- `long_rapid_same_direction_burst_reaches_exact_sum_of_deltas`
- `inverse_delta_duration_ramps_between_the_two_reference_points`
- `velocity_preserving_duration_bound_shrinks_when_moving_fast_toward_a_small_remaining_delta`
- `cubic_bezier_ease_in_out_matches_known_reference_values`

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

Phase 2 terminal tests added in `app/src/terminal/view_tests.rs` (driving `TerminalView::scroll`
and `TerminalView::advance_smooth_scroll` directly rather than through a full `BlockListElement`
paint/dispatch cycle -- the frame-driving mechanism itself is structurally identical to Phase 1's
already-tested `Manual`-axis pattern, so these focus on what's actually new: precise/flag
branching and the cancellation guard):
- `test_smooth_scroll_wheel_animates_and_settles_to_exact_target`: a non-precise notch defers
  (doesn't apply synchronously), shows partial progress mid-flight, and lands exactly where an
  immediate scroll of the same delta would have.
- `test_smooth_scroll_precise_input_applies_immediately` / `..._disabled_flag_applies_immediately`:
  precise input, and non-precise input with the flag off, both apply synchronously.
- `test_smooth_scroll_direct_action_cancels_in_flight_animation`: a direct operation (`AfterHome`)
  arriving mid-animation cancels it; advancing the animation afterward is a no-op.
- `test_smooth_scroll_animation_settles_into_follows_bottom_of_most_recent_block`: a large
  animated overshoot settles into sticky-bottom mode, not stuck at `FixedAtPosition` right at the
  boundary (this is the test that found the `heights_approx_gte` fix above).

Not covered by an automated test, and flagged rather than silently skipped: a live, real-content-
streaming-mid-animation scenario (the reasoning for why this should already work -- increments are
small and re-resolved against current state each frame -- is described above, but wasn't
empirically exercised against genuinely arriving PTY output during an active animation). Recommend
this get a pass during human/visual verification.

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
- **Phase 2:** Started after the Phase 1 controller contract merged, per the audit above. Shipped
  on branch `factory/smooth-scrolling-phase2`, stacked on `factory/smooth-scrolling-phase1` (not
  `factory/smooth-scrolling-terminal`, and including this spec amendment as its first commit
  rather than a separate spec PR, per the requester's explicit preference), as a separate PR so
  terminal work does not block Phase 1.

Within each phase, implementation and core unit-test edits touch the same state and should remain one
workstream. After local validation passes, platform verification can fan out to independent remote
agents, one current runner per relevant operating system.

## Follow-ups
- Decide whether to add a user-facing smooth-scrolling preference.
- Add system reduced-motion integration if Warp introduces a shared reduced-motion policy.
- Remove `SmoothScrolling` and its immediate fallback after rollout is stable.
