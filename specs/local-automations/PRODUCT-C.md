# Local Automations — Slice C (Row cleanup + move to cloud entry points)

## Summary
Slice C cleans up the Settings → Automations rows and adds entry points that hand off moving a local automation to an Oz cloud schedule. Rows become compact: name plus chips, a one-line humanized subtitle, a run icon, and a "···" overflow menu with Edit config, Pause/Resume, Move to cloud, and Delete. "Move to cloud" opens a two-option agent modal (Use Warp Agent, or copy a prompt for another agent) shared with the page's New and Suggested entry points. Error rows for files that failed to parse keep just the edit control. The client does no move itself: each entry point produces an agent prompt that defers all mechanics (Oz environment selection, schedule mapping, shell → agent rewrites, billing caveats, offering to disable the local file) to the create-automation skill.

This slice revises the row presentation described in Slice A/B (runner, raw cron, file path, and status all in one subtitle line).

## Goals / Non-goals
**Goals**
- Compact, scannable rows that don't truncate: chips for states, humanized schedules, no file path in the row.
- A move-to-cloud entry point per automation, opening a modal that starts a Warp agent conversation or copies a prompt for an external agent (Claude Code, Codex, ...).
- Prompts that point the agent at the automation's TOML on disk so the move works from the file's source of truth.

**Non-goals**
- An in-app move-to-cloud wizard, environment picker, or any direct Oz API calls from the client.
- Automatically disabling or editing the local TOML after a move (the agent offers that).
- Verifying plan eligibility, credits, or environment existence client-side.
- Changing the TOML schema.
- A separate confirmation dialog for Delete (delete is immediate from the overflow menu).

## Figma
Figma: none provided

## Behavior

### Row layout
1. Each valid automation row shows:
   - **Name line**: bold name, then compact chips: always a runner chip ("Warp agent" / "Shell"); plus "Missed" (warning color) when a scheduled run was missed, "Invalid schedule" (error color) when the cron doesn't parse, and "Disabled" when `enabled = false` (name also renders muted).
   - **Subtitle line**: humanized schedule, then "Next <time>" and "Last ran <time>" when known, separated by "·". No file path, no runner name, no raw status sentences.
   - **Controls**: a run (play) icon and a "···" overflow icon. Both are bare glyphs (no border, fill, or square hover background) that brighten on hover, show a pointer cursor, and have tooltips ("Run now", "More actions").
2. Rows are separated by a hairline divider with even spacing.
3. The file path is reachable via Edit config; it no longer renders in the row.
4. Error rows (files that failed to parse) keep the red "failed to load" line, the error message, and a bare edit (pencil) icon control; no run or overflow controls.
5. Everything remains gated on `FeatureFlag::LocalAutomations`.

### Humanized schedules
6. Presets render as words: `@hourly` → "Hourly", `@daily` → "Daily", `@weekly` → "Weekly", `@monthly` → "Monthly", `@yearly`/`@annually` → "Yearly".
7. Fixed-time cron shapes render as plain English: `30 18 * * *` → "Daily 6:30pm", `0 9 * * 1-5` → "Weekdays 9:00am", `0 10 * * 0,6` → "Weekends 10:00am", single weekday → e.g. "Sundays 8:00am".
8. Anything more complex (steps, ranges, day-of-month or month restrictions) falls back to the raw expression unchanged. Never show a wrong translation.

### Overflow menu
9. Clicking "···" opens a dropdown anchored below it with, in order:
   - **Edit config**: opens the automation's TOML in the editor (omitted when the source path is unknown).
   - **Pause** / **Resume**: writes `enabled = false` (Pause) or `enabled = true` (Resume) on the automation's TOML. Label is Pause when the automation is enabled and Resume when it is disabled. Omitted when the source path is unknown. Run now still works on a paused automation (existing Slice A warn+allow behavior).
   - **Move to cloud**: opens the shared two-option agent modal titled "Move \"<name>\" to cloud" with **Use Warp Agent** (opens a new tab with a Warp agent conversation seeded with the move-to-cloud prompt) and **Copy agent prompt** (copies the move-to-cloud prompt to the clipboard and shows the "Agent prompt copied" toast).
   - **Delete** (destructive styling, trash icon): removes the automation's TOML file from disk. Omitted when the source path is unknown. No confirmation dialog. Menu items use icons: pencil (Edit config), pause/play (Pause/Resume), cloud (Move to cloud), trash (Delete). No separator between items.
10. Only one overflow menu is open at a time; it closes on selection, escape, or click-away, and clicking the "···" again toggles it closed. The modal closes on selection, escape, the close button, or click-away.
11. Pause/Resume and Delete failures show an error toast; the list refreshes from disk after a successful write/delete via the existing automations directory watcher.

### Move-to-cloud prompt contract
12. The prompt asks the agent to use the create-automation skill to promote the named automation to an Oz cloud schedule and to walk the user through picking or creating an Oz environment.
13. When the automation's source path is known (always true for listed rows), the prompt references the TOML path (home-relative) so the agent reads name, schedule, and runner from disk.
14. When the source path is unknown, the prompt embeds the name, schedule, and runner (agent prompt or shell command) so it is still self-contained.
15. The external (copy) variant says "Warp local automation" so agents outside Warp have context; it assumes the create-automation skill is installed for that agent.

### Header
16. The list header is short and describes what automations are for: recurring work run by an agent or command on a schedule or in response to events, set up with a Warp agent, on this machine or in the cloud. It follows the Environments settings page descriptor pattern (what it does, then how to start one) and is not scoped to local automations. Mechanics such as "only while Warp is open" and the catch-up window live in Slice B's spec and docs, not the header.

### What must not happen
17. No cloud schedule, environment, or billable run is ever created directly by the client.
18. The local automation file is never modified by a move-to-cloud entry point.
19. Move-to-cloud entries never appear for error rows or when the feature flag is off.
20. Subtitles must not contain the file path or multi-sentence status text.
