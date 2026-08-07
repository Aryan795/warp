# REMOTE-2591: Desktop web text-input bridge for the Warp prompt

## Summary
Warp for Web must expose a focused editable browser element while its prompt is focused. This lets MacWhisper-class dictation tools, macOS Dictation, and browser text-input services attach to the prompt and insert text into the canvas-rendered editor.

The requester approved a narrow, prompt-only insertion bridge. The bridge is not a full DOM mirror of the prompt.

Figma: none provided. The bridge has no visible UI.

## Behavior
1. The bridge is active only for the main editable prompt in Warp for Web on desktop browsers. The prompt includes shell input, agent input, and follow-up input when they use the same terminal prompt editor.

2. When the Warp prompt gains focus, Warp focuses a real multiline browser text control in the same user interaction. The control stays focused for the full time that the Warp prompt owns focus.

3. When a tool such as MacWhisper activates while the prompt is focused:
   - The tool detects a focused editable text control.
   - The tool can show its normal listening UI.
   - Committed transcription is inserted into the Warp prompt.
   - One committed transcription produces one insertion.

4. The bridge is transparent and does not add visible text, a native caret, scrolling, layout changes, or a new click target. Its browser-reported bounds follow the active Warp caret so tool UI and IME candidate UI can anchor near the caret.

5. The bridge reports the native semantics of a multiline text box. Its accessible name is “Warp prompt.” It is not hidden from the browser accessibility tree.

6. The Warp editor remains the source of truth for prompt text, caret position, and selection. The bridge keeps only its input sentinel and does not expose the existing prompt contents as its value.

7. Dictated or otherwise programmatically inserted text lands at the current Warp caret.

8. If the Warp prompt has a selection, inserted text replaces that selection. Text before and after the selection remains unchanged. After insertion, the Warp caret collapses after the inserted text, following the editor’s existing insertion behavior.

9. If the prompt already contains text and has no selection, inserted text is added at the current caret. Existing text is not cleared or replaced.

10. Repeated dictation commits append or replace text at the then-current Warp caret. Resetting the bridge sentinel between commits must not move the Warp caret.

11. Hardware keyboard input continues to behave as it does before this change:
    - Printable keys insert once.
    - Enter, Tab, Escape, Backspace, Delete, arrow keys, Home, End, and Page keys keep their existing Warp behavior.
    - Warp keybindings, including Vim-mode keybindings and modified keybindings, keep working.
    - Browser shortcuts that Warp intentionally allows, such as focus-location, new-tab, close-tab, reload, and history navigation, keep working.

12. Paste inserts once through Warp’s existing paste behavior. The browser must not also paste the same text into the bridge and cause a second Warp insertion.

13. CJK and other IME input uses browser composition:
    - In-progress composition is shown through Warp’s existing marked-text UI.
    - Candidate selection does not commit intermediate text.
    - Composition commit replaces the current Warp selection or inserts at the current caret.
    - The final composition text is committed exactly once, even when the browser emits both `compositionend` and a trailing `input` event.

14. If the prompt loses focus during composition, Warp clears unfinished marked text. Warp commits text only when the browser delivered a composition commit before the blur.

15. Clicking within the prompt moves the Warp caret or changes the Warp selection using existing canvas hit testing. The bridge remains focused after the click.

16. When the user clicks another Warp surface, opens a Warp modal that takes focus, or otherwise moves Warp focus away from the prompt:
    - The bridge blurs.
    - Dictation tools no longer target the prompt.
    - The canvas or the newly focused Warp surface receives normal keyboard interaction.

17. Switching to another browser tab or application does not cause Warp to force focus back to the bridge. When the browser tab becomes active again, Warp restores the bridge only if the Warp prompt still owns focus.

18. A read-only, disabled, or viewer-only prompt does not activate the bridge.

19. Failure to create or focus the bridge does not disable the existing canvas keyboard path. Warp logs the failure without logging prompt or dictated text.

20. The existing mobile soft-keyboard behavior remains unchanged on iOS and Android. Mobile continues to use its current hidden-input path, sentinel behavior, soft-keyboard lifecycle, and viewport resize handling.

## Out of scope
- Full screen-reader reading, selection, or caret navigation is out of scope because the approved bridge does not mirror prompt contents.
- Browser autofill and password-manager integration are out of scope because the prompt is not a credential or profile form and the bridge keeps autocomplete disabled.
- Editable surfaces other than the main prompt are out of scope. Each surface needs an explicit product decision before it can opt in to a persistent DOM text-input bridge.
- Warp desktop is out of scope. The native macOS client already implements `NSTextInputClient` and exposes a native accessibility text-area role.
- Mobile dictation changes are out of scope. This work must preserve the existing mobile path without changing its behavior.
