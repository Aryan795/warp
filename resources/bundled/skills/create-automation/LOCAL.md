# Local automations

A local automation is one TOML file on disk; the whole job is writing it.

## Location

- `automations/` inside the Warp data directory, which is channel-specific: stable is `~/.warp/automations/`, other builds `~/.warp-<channel>/automations/` (e.g. `~/.warp-dev/automations/`).
- Run `ls -d ~/.warp*/` and pick the directory for the build the user is running; ask if ambiguous. Create the `automations/` subdirectory if missing.
- Filename: descriptive snake_case ending in `.toml`. One file = one automation. Ask before overwriting an existing file.

## Schema

Unknown fields are rejected - use only these fields.

```toml
# Required: display name shown in the list.
name = "Morning repo brief"

# Optional (default true): active for scheduling. Disabled automations are
# never fired; Run now works either way (disabled just shows a warning).
enabled = true

# Required: 5-field cron expression, or a preset like "@daily". Evaluated in
# local time and fired while Warp is running.
schedule = "0 9 * * 1-5"

# Exactly ONE of `cwd` or `[worktree]`:
cwd = "~/code/warp"          # must exist at run time; supports ~

# [worktree]                  # OR: run in a git worktree created/reused at
# repo = "~/code/warp"        # ~/.warp/worktrees/<repo-name>/<name>
# name = "automation-brief"
# base_branch = "main"        # optional; defaults to repo HEAD

# Required runner - one of:
[runner]
type = "warp_agent"           # unattended local agent run
prompt = "Summarize commits on main from the last 24h."

# [runner]
# type = "shell"              # terminal tab running a command
# command = "gh pr list --author @me"

# Optional run timeout. Best-effort for shell runners, when `timeout`/
# `gtimeout` is on PATH; warp_agent runs are not hard-killed.
# timeout_seconds = 1800

# Optional env vars. Currently stored but not yet applied to the run.
# [env]
# FOO = "bar"
```

Validation: `name`, `schedule`, and the runner's `prompt`/`command` must be non-empty; exactly one of `cwd`/`[worktree]`.

## Workflow

1. Clarify essentials if missing (what to do, roughly when for the `schedule` string, which directory or worktree). Use `ask_user_question` when available.
2. Write a self-contained prompt/command - each run starts fresh with no history. If the target repo has a relevant skill under `.agents/skills/`, the prompt may reference it, with a fallback so the run still works without it.
3. Write the TOML file to the correct `automations/` directory.
4. Confirm to the user: path, name, schedule string, runner - and that the automation fires on its schedule while Warp is running, with a bounded catch-up for fires missed while it was closed. Offer **Run now** (Settings → Automations) as an immediate smoke test rather than waiting for the first scheduled fire.
5. If the job needs reliability (laptop off, team visibility), mention it can be promoted to a cloud schedule later.

## Factory stages as local automations

Factory stages (triage, review, spec) make excellent local automations: the user experiences the software factory on their own repo with zero infrastructure. Adapt the corresponding skill from `warpdotdev-demos/cloud-factory-demo` into a self-contained `warp_agent` prompt:

- Condense the skill's rubric into the prompt (runs start fresh; the skill may not be installed).
- The factory skills are read-only and hand results to a workflow; a local automation applies labels/comments itself via the tracker CLI (e.g. `gh`) - say so explicitly in the prompt.
- Cap work per run (e.g. "triage at most 5 unlabeled issues") so runs stay bounded.
