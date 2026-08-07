# REMOTE-2591: Tech spec — Desktop web text-input bridge for the Warp prompt

## Context
Warp for Web renders its application shell into one browser canvas. The development shell contains no editable DOM node ([`script/wasm/dev-index.html (1-31) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/script/wasm/dev-index.html#L1-L31)). The pinned winit implementation installs keyboard listeners on the canvas itself ([`web_sys/canvas.rs @ a4e0ecb`](https://github.com/warpdotdev/winit/blob/a4e0ecb5f9626ccac9445a73dc28354b52423abc/src/platform_impl/web/web_sys/canvas.rs)). A DOM control that takes focus therefore also takes hardware keyboard events away from the current winit path.

Mobile web already has an input bridge:
- [`crates/warpui/src/platform/wasm/hidden_input.rs (1-264) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/wasm/hidden_input.rs#L1-L264) creates an off-screen `<input>`, keeps a sentinel space, and handles `input`, `compositionend`, blur, and Enter.
- [`crates/warpui/src/platform/wasm/soft_keyboard.rs (1-169) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/wasm/soft_keyboard.rs#L1-L169) owns that element and its visible/hidden state.
- [`crates/warpui/src/platform/wasm/mobile_detection/mod.rs (1-31) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/wasm/mobile_detection/mod.rs#L1-L31) identifies mobile browsers.
- [`crates/warpui/src/windowing/winit/event_loop/mod.rs (1923-2109) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/windowing/winit/event_loop/mod.rs#L1923-L2109) skips manager initialization on desktop and maps mobile bridge events to `TypedCharacters` and key events.
- [`crates/editor/src/render/element/mod.rs (598-619) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/editor/src/render/element/mod.rs#L598-L619) and [`app/src/editor/view/element.rs (284-298) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/app/src/editor/view/element.rs#L284-L298) request the mobile keyboard when an editable canvas editor is clicked.

The desktop canvas path converts winit keyboard events into a key event first and dispatches `TypedCharacters` only if the key event was not handled ([`crates/warpui/src/windowing/winit/event_loop/mod.rs (1057-1098) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/windowing/winit/event_loop/mod.rs#L1057-L1098)). This ordering preserves Warp keybindings and Vim mode. The web canvas also prevents browser defaults except for an explicit browser-shortcut allowlist, and it has a document-level paste listener ([`crates/warpui/src/platform/wasm/mod.rs (69-196) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/wasm/mod.rs#L69-L196)).

The terminal prompt constructs its shared `EditorView` with `delegate_paste_handling: true` ([`app/src/terminal/input.rs (3047-3171) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/app/src/terminal/input.rs#L3047-L3171)). This is the only editor that will opt in to the desktop bridge.

The native macOS implementation is the concrete platform reference:
- [`crates/warpui/src/platform/mac/objc/host_view.h (1-18) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/mac/objc/host_view.h#L1-L18) makes the custom-drawn host view an `NSTextInputClient`.
- [`crates/warpui/src/platform/mac/objc/host_view.m (378-450) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/mac/objc/host_view.m#L378-L450) reports `NSAccessibilityTextAreaRole` and supplies a caret rectangle to text-input services.
- [`crates/warpui/src/platform/mac/window.rs (1337-1395) @ 1445682`](https://github.com/warpdotdev/warp/blob/14456827849edeb4b263ffc76d05f7b21aa29c24/crates/warpui/src/platform/mac/window.rs#L1337-L1395) gets accessibility contents and the active caret rectangle from WarpUI.

The web implementation cannot copy AppKit protocols. Its equivalent is a focused native DOM text control with an in-viewport caret rectangle and explicit event forwarding.

## Proposed changes
### 1. Add a desktop-only DOM text-input manager
Add a desktop-specific module under `crates/warpui/src/platform/wasm/`, separate from `HiddenInput` and `SoftKeyboardManager`.

The desktop manager creates one `<textarea>` per web application:
- Element ID: `warp-desktop-text-input`.
- Native element: `<textarea>`.
- Implicit accessibility role: `textbox`.
- Attributes: `aria-label="Warp prompt"`, `aria-multiline="true"`, `autocomplete="off"`, `autocorrect="off"`, `autocapitalize="off"`, and `spellcheck="false"`.
- Value: one sentinel space with the selection collapsed after it.
- Style: `position: fixed`, transparent, one CSS pixel wide, one active line-height tall, no border, no outline, no padding, no margin, and `pointer-events: none`.
- Position: the active Warp caret in viewport CSS coordinates.

Do not use `display: none`, `visibility: hidden`, `aria-hidden`, an off-screen position, or a detached node. Those choices remove the usable accessibility target or place MacWhisper and IME UI outside the viewport.

Initialize this manager only when `is_mobile_device()` is false. Keep the current mobile manager and element unchanged when `is_mobile_device()` is true.

### 2. Add an explicit prompt opt-in and focused-view query
Add an opt-in field to `EditorOptions` for the desktop web text-input bridge. Set it to `true` only for the terminal prompt editor created in `app/src/terminal/input.rs`. Leave the default `false`.

Add a narrow `View`/`WindowCallbacks` query that answers whether the currently focused view requests the desktop bridge. The query must also confirm that the view is editable. Do not infer eligibility from `active_cursor_position()` alone because code editors, notebook editors, settings fields, and other canvas editors also report cursor positions.

Notify the WASM event loop when any of these states change:
- Focus moves to or from the opted-in prompt editor.
- The prompt becomes editable or read-only.
- The browser window becomes active or inactive.
- The active caret rectangle changes.

The desktop manager focuses the textarea only when all conditions are true:
- The browser document is active.
- The opted-in prompt view owns Warp focus.
- The prompt is editable.

On prompt blur, blur the textarea and focus the canvas only when no other DOM or browser surface has intentionally taken focus. On browser-tab blur, do not immediately refocus either element.

### 3. Make the DOM bridge the only text owner while it is focused
While the textarea is focused, winit’s canvas keyboard listeners do not receive the event. The desktop bridge must therefore forward complete keyboard events; focusing a textarea without this forwarding is a structural blocker.

Introduce a desktop bridge keyboard payload that preserves:
- Key state (`keydown` or `keyup`).
- Logical key and physical `code`.
- Text associated with the key.
- Shift, Control, Alt, and Meta modifiers.
- Repeat state.
- `isComposing`.

Convert that payload through the same event semantics as `convert_window_event`:
1. Dispatch `KeyDown` to Warp.
2. If Warp did not handle it, dispatch its characters as `TypedCharacters`.
3. Dispatch key-up and modifier transitions with the same physical-key information as the canvas path.

For non-composing hardware keyboard events, call `preventDefault()` after accepting ownership so the textarea does not also emit an `input` event. This guarantees one insertion and preserves Vim mode.

Exceptions:
- Let the browser-shortcut allowlist in `platform/wasm/mod.rs` keep browser ownership. Reuse the same predicate instead of copying the list.
- Let paste reach the bridge’s dedicated `paste` listener. Do not also dispatch a paste keybinding.
- Do not prevent the key events that the browser uses to drive an active composition. Composition owns text until commit.

Stop propagation for bridge-owned keyboard events so a future document listener cannot create a second input path.

### 4. Handle dictation and direct DOM insertion with the sentinel
Listen to `beforeinput` where useful for classification and to `input` as the authoritative post-mutation event. Mobile continues to use its current `input`-only implementation.

For a non-composing desktop `input` event:
1. Classify insertion or deletion from `inputType`.
2. Prefer `InputEvent.data` for inserted text.
3. If `data` is absent, derive inserted text from the textarea value relative to the sentinel. This fallback is required for tools that mutate the focused control and emit a generic `input` event.
4. Forward one insertion as `TypedCharacters` or one deletion as the corresponding existing Warp key event.
5. Reset the textarea to the sentinel and collapse its DOM selection after the sentinel.

The bridge never derives or changes the Warp caret from the textarea selection. `TypedCharacters` operates on the focused Warp editor’s current selection, so existing editor behavior supplies insertion-at-caret and replacement-of-selection.

Ignore unsupported `inputType` values safely and reset the sentinel. Do not log event data or textarea values.

### 5. Preserve marked-text composition and deduplicate commit
The desktop bridge must track an explicit composition session:
- `compositionstart`: enter composing state and leave the textarea value intact.
- `compositionupdate`: dispatch `SetMarkedText` with the current preedit text and a selection range derived from the textarea composition selection.
- `compositionend`: dispatch `ClearMarkedText`, then one `TypedCharacters` commit when the final data is non-empty.
- Trailing `input`: suppress the browser’s matching post-`compositionend` event so it cannot commit the same text twice.
- Blur or cancellation before commit: dispatch `ClearMarkedText` without inserting unfinished text.

Reset the sentinel only after commit, cancellation, or blur. Keep the existing editor `IMEOpen` keymap context behavior by using the existing `SetMarkedText` and `ClearMarkedText` events.

### 6. Route paste exactly once
Add a `paste` listener to the focused desktop textarea:
1. Read the same plain-text and HTML clipboard payload used by `add_paste_listener`.
2. Call `preventDefault()` so the sentinel value is not mutated.
3. Stop propagation so the document-level paste listener does not see the same event.
4. Send the existing `CustomEvent::Clipboard(ClipboardEvent::Paste(...))`.

The existing `handle_clipboard_event` then dispatches `StandardAction::Paste`, preserving the terminal input’s delegated paste behavior.

### 7. Keep caret geometry and focus classification current
Reuse `CursorInfo` from the active-cursor callback to position the textarea. Convert the canvas-relative logical rectangle to viewport CSS coordinates using the canvas bounding rectangle. Update position when `ActiveCursorPositionUpdated` fires and immediately before focusing.

Treat focus transfer between the canvas and `warp-desktop-text-input` as internal to the same Warp web window. Do not emit an application resign/activate transition for this transfer. A real `window` or `document` blur still updates application activity.

If creating, positioning, or focusing the textarea fails:
- Leave or restore canvas focus.
- Keep the existing canvas keyboard path active.
- Log only the operation and browser error object. Do not log prompt text, input data, or clipboard content.

## Decisions
### Backing element: `<textarea>` with native textbox semantics
Chosen: a native `<textarea>` with implicit `textbox` role, `aria-multiline="true"`, and accessible name “Warp prompt.”

Advantages:
- Matches the multiline Warp prompt.
- Provides stable `input`, composition, value, and selection APIs in Chrome and Safari.
- Exposes a native editable control to macOS/browser accessibility services.

Rejected alternatives:
- `<input type="text">` is single-line and gives browsers the wrong semantic model for multiline prompts.
- `contenteditable` has more browser-specific selection and mutation behavior and provides no benefit for a sentinel-only sink.
- ARIA on the canvas does not make it an editable text control and cannot receive dictation insertion.

### Value model: sentinel input sink, not a full mirror
Chosen: keep the current mobile-style sentinel model. The Warp editor remains the only prompt-content and selection model.

Advantages:
- Solves the approved dictation and IME insertion scope without a bidirectional editor synchronization protocol.
- Avoids UTF-16/UTF-8 range mapping, multiple-selection mapping, stale DOM value races, and large prompt copies.
- Lets `TypedCharacters` reuse the editor’s current caret and replacement behavior.

Trade-off:
- Screen readers and dictation tools cannot read the existing prompt through the textarea or navigate its DOM value.

Rejected alternative:
- Mirroring the full prompt would improve accessibility, but it requires value, selection, caret, edit-range, undo, and multi-cursor synchronization. The requester explicitly approved the narrow scope (“good with narrow”).

### Focus lifetime: persistent while the prompt owns focus
Chosen: keep the textarea focused for the entire prompt-focus lifetime.

Advantages:
- Global-hotkey dictation tools can attach without an extra click or timing window.
- Native browser input and IME use a stable composition target.
- Focus behavior is predictable after mouse and keyboard caret movement.

Rejected alternative:
- On-demand attachment cannot know that an external dictation tool is about to insert and can miss the activation event.

### Product scope: prompt dictation and text input
Chosen:
- MacWhisper-class tools.
- macOS/browser dictation that inserts into a focused editable control.
- Hardware keyboard, paste, and CJK/IME compatibility while the bridge owns focus.
- The main terminal prompt only.

Deferred:
- Autofill and password managers because this is not a form or credential field.
- Full screen-reader content and selection support because the bridge is not a value mirror.
- Other editable Warp surfaces because they need separate opt-in, labels, focus rules, and validation.
- Other operating-system dictation tools because this issue and required validation are macOS-specific.

### Mobile isolation: separate manager and unchanged behavior
Chosen: keep the current mobile `HiddenInput` and `SoftKeyboardManager` behavior unchanged. Share pure event-classification helpers only when doing so does not change mobile event order or DOM attributes.

Rejected alternative:
- Replacing both paths with one generalized element creates unnecessary regression risk for soft-keyboard activation, Android Backspace, iOS viewport sizing, and keyboard dismissal.

## Assumptions
- **Assumption:** MacWhisper and macOS Dictation attach to an accessibility-visible, focused textarea whose value contains only a sentinel. The required Chrome and Safari manual tests must confirm this assumption before implementation can ship.
- **Assumption:** Shell input, agent input, and follow-up input use the terminal prompt `EditorView` constructed in `app/src/terminal/input.rs`. A prompt flow that uses another editor is outside this ticket until it opts in explicitly.
- **Assumption:** A one-pixel-wide, line-height-tall textarea positioned at `CursorInfo` provides a usable anchor for MacWhisper and IME UI. If browser testing disproves this, the implementation may enlarge the transparent bounds around the caret without changing the value model or scope.

## Risks and mitigations
- **MacWhisper ignores a fully transparent textarea.** Keep the node attached, in viewport, focusable, and accessibility-visible. Validate with the exact Loom workflow before considering the work complete. If `opacity: 0` is not exposed by a required browser, use a visually clipped/transparent technique that remains in the accessibility tree; do not move the node off-screen.
- **Hardware keys stop working when the textarea takes focus.** Treat complete keyboard forwarding as required infrastructure, not a follow-up. Test Vim mode and modified keybindings.
- **Text inserts twice.** Give each event class one owner: forwarded key events for hardware typing, `input` for dictation/direct insertion, composition events for IME, and `paste` for clipboard paste.
- **Composition commits twice.** Track composition state and the expected trailing `input` event explicitly.
- **Focus loops steal browser focus.** Distinguish internal canvas/textarea transfers from real window blur and never refocus while the document is inactive.
- **Tool UI appears in the wrong location.** Position the textarea at the current Warp caret rather than using the mobile off-screen style.
- **Mobile soft keyboard regresses.** Do not change mobile manager construction, attributes, focus lifecycle, sentinel reset order, or viewport listener behavior. Run mobile manual regression checks.
- **Bridge expands to every editor accidentally.** Require an explicit `EditorOptions` opt-in and test that non-prompt editors return no active bridge request.

## Testing and validation
### Automated tests
Add pure unit tests for the desktop event reducer and focus policy. Keep browser-independent classification logic outside direct `web_sys` calls so host tests can exercise it.

Required cases:
- Unmodified printable hardware key: one `KeyDown`, then one fallback `TypedCharacters`; no DOM `input` insertion.
- Handled keybinding and Vim-normal key: `KeyDown` only; no `TypedCharacters`.
- Browser-allowed shortcut: no Warp event and no prevented browser default.
- Dictation-style `input` with `data`: one `TypedCharacters`.
- Generic `input` without `data`: extract one insertion from the sentinel-relative value.
- Backspace and forward Delete map to distinct existing Warp key events.
- Paste produces one `ClipboardEvent::Paste` and suppresses document propagation.
- Composition update sends marked text.
- `compositionend` plus trailing `input` commits once.
- Blur during composition clears marked text without a phantom commit.
- Prompt focus activates the desktop manager; other editor focus does not.
- Inactive document never triggers a refocus.
- Mobile event-mapping tests in `soft_keyboard_tests.rs` continue to pass without changed expectations.

Run:
- `cargo nextest run -p warpui`
- `cargo nextest run -p warp`
- `cargo clippy --locked --target wasm32-unknown-unknown --profile release-wasm-debug_assertions -- -D warnings`
- `./script/wasm/bundle --channel oss --nouniversal --check-only`
- `./script/format --check`

### Manual browser and platform matrix
**macOS + Chrome stable — required**
- Reproduce the Loom flow with MacWhisper and the requester’s left-Option hotkey.
- Capture a video that shows the MacWhisper waveform UI appearing, dictated text entering the prompt, and the prompt submitting normally.
- Repeat with an empty prompt, a caret in the middle of existing text, and an existing selection.
- Verify ordinary typing, Enter, arrows, Backspace, Delete, Cmd-A, Cmd-C, Cmd-X, Cmd-V, undo/redo, and a Vim-mode insertion.
- Verify paste inserts once.
- Verify a Japanese or Chinese IME shows marked text and commits once.
- Verify Cmd-L, Cmd-T, Cmd-W, reload, and browser history shortcuts retain their intended browser behavior.
- Click outside the prompt and verify MacWhisper no longer targets it.

**macOS + Safari stable — required**
- Repeat the MacWhisper empty/caret/selection cases.
- Repeat typing, paste, IME composition, blur, and browser-shortcut checks.
- Capture the successful MacWhisper interaction in the same video or a second video.

**iOS Safari + Android Chrome — regression check**
- Focus the prompt and verify the soft keyboard opens.
- Type text, Backspace, press Enter, dismiss the keyboard, and refocus the prompt.
- Verify viewport resizing and canvas interaction still work.

### Explicitly not validated
- Firefox and Edge.
- Dictation tools on Windows or Linux.
- Autofill and password-manager behavior.
- Full VoiceOver or other screen-reader reading and caret navigation.
- Warp for Web editors other than the main terminal prompt.
- Native Warp desktop input.

The implementation is complete only when the automated checks pass and the Chrome and Safari MacWhisper videos prove the original Loom failure is fixed.
