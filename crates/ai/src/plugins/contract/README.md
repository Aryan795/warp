# Vendored cross-repo contracts

**These files are copies. `warpdotdev/warp-server` is canonical — do not edit them here.**

| File | Canonical location in `warpdotdev/warp-server` |
| --- | --- |
| `factory_plugin_runtime_contract.json` | `logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json` |

## Why a copy rather than a shared artifact

The Factory plugin environment contract is produced by warp-server and consumed by this client.
It previously lived as prose in both repositories, the two implementations were written
independently against it, and they disagreed: the server discarded the Factory UID while the
client appended its own unrelated local layout, so neither side produced the specified path.

We have no build-time mechanism for sharing an artifact across the two repositories, so the
mechanism is deliberate, visible duplication. warp-server asserts the producing half against its
copy; this crate asserts the consuming half against this one. A divergence then shows up as a diff
between two committed files that a reviewer can see, instead of as plugin data quietly landing in
the wrong directory at runtime.

## Updating

Change the canonical file in warp-server first, then re-copy it here verbatim in the same change
that updates this side's behavior. If the two copies differ, warp-server wins.

## Known delta

The contract's `scope_values` are written as `agent/<agent-name>` and `automation/<automation-name>`.
This client emits those two segments separately and reduces the author-supplied name to a single
safe segment before joining, because an agent name comes from a Factory repository and a value such
as `../../etc` would otherwise climb out of the durable root. A conformant name sanitizes to itself,
so both of the contract's worked examples compose identically on this side; the transformation is
observable only for a name that could not have been used safely anyway. The sanitization rule is
documented on `filesystem_safe_segment` and is pending inclusion in the canonical file.
