---
name: create-automation
description: Creates Warp automations, routing each request to the right plane - a local automation TOML on the user's machine, an Oz cloud schedule, or a GitHub-triggered software factory (issue triage, speccing, implementation, PR review). Use when the user wants a recurring agent job, an event-triggered agent, or a local automation.
---

# create-automation

One automation request, three planes:

- **Local** - a TOML file on this machine, listed in Settings → Automations, run via **Run now**. (morning repo brief, stale branch cleanup, nightly local triage)
- **Cloud** - an Oz scheduled agent; fires with the laptop off. (weekly bug triage, dependency audits)
- **GitHub trigger** - Oz agents fired by GitHub events, up to the full software factory: triage → spec → implement → review.

## Route

Invoked without a task (bare `/create-automation`): first ask what the automation should do, in plain text, seeded with a few ideas spanning the planes - e.g. a morning brief of commits and PRs, stale branch cleanup, a nightly dependency audit, issue triage on new issues, automated PR review. Glance at the current repo to make suggestions concrete. Only then route.

Explicit intent routes directly, with no question:
- "local automation", "on my machine", "automation TOML" → Local
- "cloud schedule", "even when my laptop is off", "Oz" → Cloud
- "when an issue is opened/labeled", "on PR", "software factory" → GitHub trigger

Otherwise ask with `ask_user_question`, one option per plane with an example each, and set the recommended option by judging where the task naturally lives:
- Work keyed to repo events (issue triage, PR review, spec/implement on label) → recommend GitHub trigger.
- Recurring work needing reliability or team visibility (laptop-off crons, shared audits) → recommend Cloud.
- Personal, machine-bound, or exploratory work → recommend Local; it is the cheapest trial and promotes to Cloud later.

## Pick the agent

For agent-style local jobs, ask which agent runs the automation:
- **Warp agent** (`warp_agent` runner): Warp AI usage, unattended permission profile handled automatically.
- **Personal CLI** (`claude`, `codex`, `gemini`): a `shell` runner invoking the CLI headless (`claude -p '...'`, `codex exec '...'`). User's own subscription; include permissive autonomy flags only with their explicit okay.

Recommend the harness running this conversation (Claude Code → `claude -p`, Codex → `codex exec`, Warp → `warp_agent`); confirm the CLI exists with `which` first. Plain commands are always `shell`. Cloud and GitHub trigger always run Warp/Oz agents.

## Local

Follow [`LOCAL.md`](LOCAL.md): file location, TOML schema, workflow, and adapting factory stages (triage, review, spec) into self-contained local prompts.

## Cloud

1. Confirm prerequisites: eligible plan with credits, and an Oz environment for repo-touching tasks.
2. Create the schedule with `oz schedule create` (read the `oz` skill for CLI specifics) or at https://oz.warp.dev/schedules.
3. Map the same fields as local: name, cron, self-contained prompt. Offer a Run now test from the schedule page before relying on the cron.

Promoting an existing local automation: lift `name`, `schedule`, and the prompt from its TOML. For `shell` runners, offer a rewrite to a Warp agent prompt and explain what changes, including that cloud runs bill Warp credits rather than a personal CLI subscription. After a successful promote, offer to set `enabled = false` in the local file.

## GitHub trigger

Install and invoke the guided setup skill from the canonical factory repo:

```bash
npx skills add warpdotdev-demos/cloud-factory-demo --skill oz-cloud-factory-demo --agent warp --yes
```

Run `oz-cloud-factory-demo` with the target repo (URL, `owner/repo`, or local path). It installs the factory skills and GitHub Actions workflow templates, configures Oz auth, and gates billable test runs. Mention up front: needs a `WARP_API_KEY` repo secret and Oz credits. For a single stage (e.g. just PR review), copy only that workflow and skill.
