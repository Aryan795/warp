# Child-run deep links in the web session viewer

## Summary
The proposed web session viewer keeps an orchestrator route in the address bar while the user views a child run. The selected child is encoded in a URL fragment, so refreshes and copied links reopen the orchestration viewer with that child selected. Five product choices remain recommendations pending requester confirmation.

## Problem
Today a child-pill click can replace the root orchestrator URL with the child's own `/conversation/<id>` or `/session/<id>` URL. A refresh or copied link then opens the child without its orchestration context. A narrow stopgap preserves the root route but drops the selected child from the URL. This specification replaces that stopgap behavior with a root route plus a child-selection fragment.
## Open decisions awaiting requester confirmation
1. **Nested children — recommended: top-level root.** A direct link to a child at any depth canonicalizes to the top-level orchestrator, not the immediate parent. This gives the whole tree one stable viewer URL. Alternative: canonicalize one parent hop at a time.
2. **Standalone escape hatch — recommended: ship in the first implementation.** `?view=standalone` suppresses only child-to-orchestrator canonicalization and survives automatic `/conversation` to `/session` redirects. This keeps a direct-child debugging and sharing path. Alternative: defer the escape hatch.
3. **Browser history — recommended: replace cold redirects, push pill selections.** This avoids redirect loops while making Back traverse child selections and recover the orchestrator. Alternative: replace pill selections and keep Back unaware of them.
4. **Anchor identity — recommended: `#child=<run-id>`.** A run ID survives a run's transition between a live session and a stored conversation. Alternative: put a conversation or session identifier in the fragment.
5. **Access fallback — recommended: keep the direct child viewer.** If the viewer can access the child but not the orchestrator, do not redirect or disclose the orchestrator identifier. Alternative: show an access error after redirecting.

The behavior below states these recommendations as the proposed contract. It is not approved until the requester confirms or changes all five choices.

## Behavior

### URL shape and pill navigation
**Recommended behavior pending confirmation of anchor identity.**
1. The stable viewer URL is the root orchestrator's route:
   - `/conversation/<root-conversation-id>`
   - `/session/<root-session-uuid>`
2. When a child run is selected, the viewer appends the child's run ID as a URL fragment:
   - `/conversation/<root-conversation-id>#child=<child-run-id>`
   - `/session/<root-session-uuid>#child=<child-run-id>`
3. The URL fragment represents only the selected run. It does not identify the transcript format or the current child session.
4. Clicking a child pill keeps the root route and its supported query parameters unchanged. It changes only the `child` fragment.
5. Clicking a different child pill replaces the fragment value with that child's run ID.
6. Clicking the root orchestrator pill removes the `child` fragment.
7. Clicking the selected pill is a no-op. It does not create a duplicate browser-history entry.
8. A child pill without a durable run ID remains selectable, but the viewer leaves the URL unanchored. A copied or refreshed URL then reopens the root orchestrator.

### Opening and refreshing links
9. Loading a root URL with `#child=<run-id>` first loads the root orchestration viewer. After the viewer has indexed the root's children, it selects the child whose run ID matches the fragment.
10. The viewer keeps the normal root loading state while it discovers and materializes the selected child. It does not render the child as a standalone viewer.
11. Refreshing a valid anchored root URL restores the same root and selected child.
12. Copying a valid anchored root URL and opening it in another authorized browser restores the same root and selected child.
13. If the run ID is stale, malformed, outside the root's orchestration tree, or unavailable after initial orchestration hydration settles, the viewer shows the root orchestrator with no child selected. It removes the invalid fragment with a history replacement.
14. The viewer does not use a timeout to decide that an anchor is stale. It waits for an explicit completion signal from initial orchestration hydration.
15. Automatic routing between a live session and a stored conversation preserves the child fragment. For example, `/session/<root-session>#child=<run>` may become `/conversation/<root-conversation>#child=<run>` after the session ends.

### Existing direct child links
**Recommended behavior pending confirmation of root canonicalization.**
16. Existing `/conversation/<child-conversation-id>` and `/session/<child-session-uuid>` links continue to work.
17. By default, opening an orchestration child's direct URL resolves the child's ancestry and replaces the URL with the top-level root orchestrator route plus `#child=<child-run-id>`.
18. An arbitrarily deep child redirects to the top-level root, not its immediate parent. The resulting root viewer selects the originally requested child.
19. A cold redirect selects the root's live `/session` route when a reachable, authorized root session exists. Otherwise it selects the authorized root `/conversation` route when a stored transcript exists.
20. If the server cannot resolve a complete, valid ancestor chain to a routable root, the direct child viewer opens normally. The server does not redirect to an intermediate ancestor.
21. A non-orchestration conversation or session opens normally and does not gain a child fragment.

### Access control and failures
**Recommended behavior pending confirmation of the access fallback.**
22. The server returns a root redirect only when the same viewer can access both the requested child and the selected root route.
23. If the viewer can access the child but cannot access the root, the child opens as a standalone viewer at its original URL. The response does not disclose a root run ID, conversation ID, session UUID, or route.
24. Missing, cyclic, over-depth, or partially deleted ancestor chains use the same standalone-child fallback. These conditions do not expose a partial ancestry result.
25. Transient route-resolution failures do not block an otherwise accessible child. The child opens standalone and the client may log the failure without showing internal details.
26. Normal access checks still apply after a redirect. If access changes between resolution and navigation, the destination uses the existing viewer access-error behavior.

### Standalone escape hatch
**Recommended behavior pending confirmation of first-implementation scope.**
27. The first implementation supports `?view=standalone` on direct child URLs:
   - `/conversation/<child-conversation-id>?view=standalone`
   - `/session/<child-session-uuid>?view=standalone`
28. `view=standalone` suppresses only child-to-root canonicalization. It does not suppress the existing automatic redirect between the same run's live `/session` route and stored `/conversation` route.
29. Automatic `/conversation` to `/session` and `/session` to `/conversation` redirects preserve `view=standalone`.
30. `view=standalone` is case-sensitive. Unknown `view` values use the default root-canonicalization behavior.
31. If a standalone child viewer exposes descendants, selecting a descendant keeps the standalone child's route and query string as the viewer base, then adds the selected descendant's `#child=<run-id>` fragment.

### Browser history
**Recommended behavior pending confirmation of history semantics.**
32. A cold direct-child-to-root redirect replaces the current history entry. Browser Back returns to the page before the child link instead of reopening the child and redirecting again.
33. A user-initiated pill selection pushes one browser-history entry.
34. Browser Back and Forward traverse prior pill selections, including the unanchored root state. Applying a history entry changes the selected pill without adding another entry.
35. The in-view back action returns from a child to the root and records the unanchored root state consistently with a root-pill selection.
36. Focus changes, session-join events, transcript loading, and other non-navigation state updates do not create browser-history entries.

## Recommended decisions awaiting confirmation
- Use `#child=<run-id>` because a run ID survives live-session and stored-conversation route changes.
- Canonicalize direct child links to the top-level root to keep one stable viewer for the complete orchestration tree.
- Ship `?view=standalone` in the first implementation to preserve a direct-child debugging and sharing path.
- Replace cold redirects and push pill selections so Back works without redirect loops.
- Fail parent resolution closed so an authorized child remains usable instead of redirecting to an inaccessible parent.

## Assumptions
- Orchestration children created by current run infrastructure have durable run IDs.
- Existing viewer access-error screens remain the fallback for an authorization race after route resolution.

## Out of scope
- Redesigning the orchestration pill bar, transcript viewer, pane swap, or orchestration hierarchy.
- Adding new controls for execution, messaging, pinning, or pane management.
- Changing native desktop URL behavior.
- Redirecting to the nearest valid ancestor when the root cannot be resolved.
- Supporting arbitrary fragment keys beyond the exact `child` key.
