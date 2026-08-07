# Agent Plugins conformance corpus

This directory is the **canonical** corpus of Agent Plugins 1.0.0 validation
cases for Warp. Both validators run it:

- Go — `logic/factoryfile` (Factory sync), driven by `TestAgentPluginsConformanceCorpus`.
- Rust — the Warp client's plugin loader, which vendors a verbatim copy of this
  directory with a provenance header pointing here.

The two implementations validate the same documents independently. Cross-repo
build-time sharing is not available, so the mechanism against drift is a visible
duplicate plus a committed expectations file on each side: a divergence shows up
as a diff a reviewer can see, rather than as a package that sync accepts and the
runtime then disables.

## Layout

- `cases.json` — the index. Every fixture must appear in it exactly once, and
  every entry must point at a file that exists; the Go driver asserts both, so
  a fixture cannot be added without declaring its expectation.
- `manifest/<name>.json` — a root `plugin.json` document.
- `mcp/<name>.json` — a plugin root `mcp.json` document.

Each `cases.json` entry has:

| Field    | Meaning                                                            |
| -------- | ------------------------------------------------------------------ |
| `name`   | Stable case identifier, unique within its `kind`.                   |
| `kind`   | `manifest` or `mcp` — selects which validator the driver invokes.   |
| `path`   | Fixture path relative to this directory.                            |
| `expect` | `valid`, `warn`, or `invalid`.                                      |
| `reason` | The rule being pinned, cited by Agent Plugins section number.       |

## Expectations

- `valid` — the document is accepted and produces no diagnostic at all.
- `warn` — the document is accepted and produces at least one non-blocking
  diagnostic. This is the standard's report-and-continue behavior: an unknown
  top-level manifest field, a non-object `extensions`, and the optional `sse`
  transport. An implementation that rejects one of these is non-conformant.
- `invalid` — the document is rejected with at least one blocking diagnostic.

Diagnostic codes and message wording are deliberately **not** part of the
contract: the two implementations have different diagnostic vocabularies. Only
the accept/warn/reject outcome is compared.

## Warp-specific cases

Two `mcp` cases pin a Warp rule rather than an Agent Plugins one, and are marked
as such in their `reason`: a `managed` entry and the Warp Factory MCP `$schema`
are both invalid inside a plugin root. They live here because both
implementations must enforce them at the same boundary — a plugin package must
not be able to reach a managed Warp MCP server.

## Adding a case

Add the fixture file and its `cases.json` entry in the same change, then run the
Go driver. Land the identical addition on the Rust side; the corpus is only
worth having while both sides run all of it.
