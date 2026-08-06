# APP-5237: Agent Plugins 1.0.0 Technical Design
## Context
The [product spec](PRODUCT.md) defines client and Factory behavior. This technical design is pinned to:
- Warp client commit [`7a6044bd`](https://github.com/warpdotdev/warp/tree/7a6044bd5377d708ab1d3767ece505a49d232aed).
- Warp server commit [`d35b195a`](https://github.com/warpdotdev/warp-server/tree/d35b195a9bee8b512f860df1dcb77619ecf278d9).
- Published [Agent Plugins 1.0.0](https://github.com/agentplugins/agent-plugins-spec/blob/main/spec/1.0.0.md).

The client has no plugin package abstraction today.

- [`crates/ai/src/skills/skill_provider.rs:106`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/crates/ai/src/skills/skill_provider.rs#L106) defines flat skill-provider precedence. `.agents` ranks before `.warp`.
- [`app/src/ai/skills/file_watchers/skill_watcher.rs:92`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/skills/file_watchers/skill_watcher.rs#L92) watches home and repository skills.
- [`app/src/ai/skills/skill_manager.rs:106`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/skills/skill_manager.rs#L106) scopes skills by home, current repository, or all cloud repositories.
- [`app/src/ai/skills/resolve_skill_spec.rs:103`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/skills/resolve_skill_spec.rs#L103) resolves explicit CLI skill names and already reports repository ambiguity.
- [`app/src/ai/mcp/file_mcp_watcher.rs:122`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/mcp/file_mcp_watcher.rs#L122) watches provider-specific file-based MCP configuration.
- [`app/src/ai/mcp/file_based_manager.rs:18`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/mcp/file_based_manager.rs#L18) owns parsed file-based installations and scope.
- [`app/src/ai/mcp/file_based_manager.rs:345`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/mcp/file_based_manager.rs#L345) preserves current MCP auto-start behavior: global Warp always, enabled global third-party in GUI, and never project-scoped.
- [`crates/warp_core/src/paths.rs:62`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/crates/warp_core/src/paths.rs#L62) owns Warp's channel-aware home config directory.
- [`crates/warp_core/src/paths.rs:208`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/crates/warp_core/src/paths.rs#L208) intentionally separates TUI global MCP configuration from GUI configuration.

Factories currently have direct flat skills and managed MCP references only.

- [`logic/factoryfile/path.go:8`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/path.go#L8) defines factory and agent skill paths.
- [`logic/factoryfile/v1alpha1.go:28`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/v1alpha1.go#L28) admits `mcpServers` in factory YAML and agent/automation frontmatter.
- [`logic/factoryfile/v1alpha1.go:543`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/v1alpha1.go#L543) accepts only `warpId` entries.
- [`logic/factoryfile/resolution/resolution.go:643`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/resolution/resolution.go#L643) merges managed MCP by name and rejects conflicting IDs.
- [`logic/factoryfile/projector/agent.go:370`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/projector/agent.go#L370) validates managed MCP against team scope before projection.
- [`logic/factoryfile/projector/automation.go:35`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/projector/automation.go#L35) validates automation MCP overrides.
- [`logic/ai/ambient_agents/workers/common/factory_skill_dirs.go:22`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/ai/ambient_agents/workers/common/factory_skill_dirs.go#L22) derives applicable Factory skills at dispatch and sends them through `WARP_SKILL_DIRS`.
- [`logic/ai/ambient_agents/workers/common/task_utils.go:816`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/ai/ambient_agents/workers/common/task_utils.go#L816) sends effective managed MCP to the client through `--mcp`.
## Proposed changes
### 1. Shared client package model
Add `crates/ai/src/plugins/` with no UI or filesystem-watcher dependency:
- `manifest.rs` contains versioned manifest types and semantic validation.
- `mcp.rs` contains versioned Agent Plugins MCP types and per-entry validation.
- `package.rs` contains `PluginPackage`, `PluginComponent`, diagnostics, canonical identities, and failure boundaries.
- `paths.rs` performs filesystem-resolved containment checks.
- `schema/1.0.0/` vendors the published immutable manifest and MCP schemas.

The loader selects a parser by exact canonical `$schema`. It never performs a network fetch. Semantic checks supplement JSON Schema for path containment, URL origin, case-insensitive duplicate headers, command-token rules, and version matching.

Use a structured identity instead of overloading a display string:

```rust
pub struct PluginInstanceId {
    pub scope: PluginScopeId,
    pub source: PluginSourceId,
    pub manifest_name: String,
}

pub struct PluginSourceId {
    pub kind: PluginSourceKind,
    pub stable_identity: String,
}

pub struct PluginComponentId {
    pub plugin: PluginInstanceId,
    pub kind: PluginComponentKind,
    pub local_name: String,
}
```

`PluginSourceKind` is `AgentsDirectory`, `WarpDirectory`, or `FactoryRepository` in v1. `stable_identity` identifies the user root, repository, or Factory source without depending on a mutable version. Keep source identity opaque at component boundaries so a later remote source kind does not change component identity. `PluginScopeId` distinguishes user, repository, factory, agent, and automation instances. UI/model adapters render `<plugin>:<component>`.

Do not treat plugins as one more row in `SKILL_PROVIDER_DEFINITIONS`. That list describes flat skill roots, while a plugin requires manifest-first loading and package-level shadowing.
### 2. Client discovery and watching
Add `app/src/ai/plugins/`:
- `plugin_watcher.rs` watches configured search roots and detected repositories.
- `plugin_manager.rs` owns candidate snapshots, precedence, active packages, diagnostics, and component registration.
- `plugin_data.rs` resolves persistent data paths and prepares stdio environments.
- `factory_mcp.rs` parses the distinct Warp Factory MCP schema supplied by a worker.

`PluginWatcher` reuses:
- `HomeDirectoryWatcher` to notice `.agents` creation.
- `WarpManagedPathsWatcher` for the channel-aware Warp home plugin root.
- `RepoMetadataModel` and repository subscribers for project roots.

Extend `WarpManagedPathsWatcher` with a precise recursive root for `<warp-home-config-dir>/plugins`. Preserve its existing `worktrees` exclusion. Do not recursively watch the complete Warp home config directory.

For each search root:
1. Enumerate immediate child directories.
2. Resolve the candidate root and root manifest.
3. Parse `plugin.json`.
4. Build the package snapshot.
5. Parse standard components independently.
6. Publish one generation-tagged update so stale asynchronous parses cannot replace newer state.

Package-level parse, unsupported-component, and shadowing diagnostics emit structured log events with stable diagnostic codes, source path, and scope. Invalid or ambiguous explicit skill invocation returns the matching codes and candidate identities. Component-level skill and MCP status continues through existing Skills and MCP models.

Precedence is a tuple:

```text
(scope rank: repository < user, provider rank: .agents < .warp)
```

Lower rank wins. Same-rank cross-repository collisions are ambiguous. Shadowing occurs after manifest validation and by manifest `name`, not child-directory name.
### 3. Skill integration
Add a plugin ingestion API to `SkillManager`. It accepts parsed skills with explicit `PluginComponentId` and owning scope instead of deriving provider and parent from a flat path.

Extend `ParsedSkill` or `SkillDescriptor` with optional plugin provenance and a runtime invocation name. Keep the Agent Skills frontmatter name unchanged. The runtime invocation name is qualified.

Update:
- Skill catalog serialization to send the qualified invocation name.
- Slash-command and explicit `--skill` resolution to accept `<plugin>:<skill>`.
- `SkillReference` with a plugin component variant, or an equivalent stable structured reference. Do not encode identity only in a mutable path.
- Ambiguity errors to include flat and plugin candidates.
- GUI and TUI skill lists to show the qualified plugin skill name and existing source detail.

Unqualified resolution first gathers every active candidate. It returns a match only when exactly one candidate has that local name. Existing repository qualification remains available for flat skills and can disambiguate cross-repository plugin sources before plugin qualification.
### 4. MCP integration
Do not parse plugin MCP through the native provider parser. Agent Plugins has a different closed schema, required `type`, and different placeholder rules.

Map each valid plugin server into a `TemplatableMCPServerInstallation` plus immutable launch context:

```rust
pub struct PluginMcpLaunchContext {
    pub component_id: PluginComponentId,
    pub plugin_root: PathBuf,
    pub plugin_data: PathBuf,
    pub discovery_scope: FileBasedMCPServerScope,
    pub source: PluginSourceId,
}
```

Extend `FileBasedMCPManager` with a plugin source kind and registration API. Its stable hash must include `PluginComponentId` and normalized configuration. Two packages with identical JSON must not collapse into one installation.

Preserve `FileBasedMCPManager::auto_start_decision` by mapping:
- User Warp plugin source to `GlobalWarp`.
- User Agents plugin source to `GlobalThirdParty`.
- Every repository plugin source to `ProjectScoped`.
- Factory runtime source to an explicit runtime-start policy described in section 7.

Plugin `mcp.json` stdio servers are the only processes that the plugin loader launches on a package's behalf. The spawner must:
1. Create the dedicated plugin data directory.
2. Expand exact `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` occurrences once in `args`, `env` values, and `cwd`.
3. Resolve and contain `command` and `cwd`.
4. Overlay configured `env`.
5. Set authoritative `PLUGIN_ROOT` and `PLUGIN_DATA` last.
6. Launch `command` as one executable token with a separate argument vector.

For HTTP:
- Parse absolute URL semantics before mapping to the native transport.
- Reject userinfo, fragments, non-HTTPS non-loopback origins, invalid duplicate headers, and redirect forwarding to another origin.
- Keep URL and headers literal.

The manager exposes plugin server provenance to GUI/TUI MCP models. Model tool metadata uses structured installation ID plus native tool name. Display adapters render `<plugin>:<server>/<tool>` without changing the MCP request's tool name.
### 5. Persistent plugin data
Add a `PluginDataLocator` interface:

```rust
pub trait PluginDataLocator {
    fn data_dir(&self, instance: &PluginInstanceId) -> Result<PathBuf>;
}
```

Local data lives under the active frontend's `warp_core::paths::data_dir()/plugins/data/<instance-key>`. `instance-key` is a filesystem-safe hash of frontend identity, stable source identity, scope, and manifest name. It excludes manifest version and component content. GUI and TUI therefore discover the same packages but do not share writable plugin state or running processes.

Skill-bundled scripts do not use `PluginDataLocator`. The skill content can direct the model to run one through the ordinary shell-command action. `BlocklistAIPermissions::can_autoexecute_command` applies the active execution profile, allowlist, denylist, risk classification, and user approval behavior. The plugin loader does not spawn the script and does not inject `PLUGIN_ROOT` or `PLUGIN_DATA`.

Factory workers pass an absolute `WARP_PLUGIN_DATA_ROOT`. The client appends a stable key containing Factory UID, scope kind/name, and plugin name.

- Warp-hosted workers mount this root from environment-persistent storage.
- Self-hosted Docker workers bind it to a configured durable host path.
- Self-hosted direct workers use a configured worker-local durable path and inherit that backend's existing process-isolation boundary.
- Dispatch fails before starting a Factory plugin MCP stdio server when the worker cannot provide a writable persistent root. An ephemeral fallback would violate Agent Plugins conformance.

Concurrent processes can share one plugin instance data directory. Warp guarantees directory persistence, not application-level locking.
### 6. Factory source model and validation
Extend `logic/factoryfile` source classification with:
- Factory `plugins/<child>/plugin.json`.
- Agent `agents/<name>/plugins/<child>/plugin.json`.
- Automation `automations/<name>/plugins/<child>/plugin.json`.
- Factory `mcp.json`.
- Agent `agents/<name>/mcp.json`.
- Automation `automations/<name>/mcp.json`.

Add canonical source records:

```go
type PluginResource struct {
    Scope        PluginScope
    OwnerName    string
    ManifestName string
    RootPath     string
    Digest       string
}

type FactoryMCPFile struct {
    Scope     MCPScope
    OwnerName string
    Path      string
    Servers   map[string]FactoryMCPServerEntry
}
```

The tree parser validates plugin packages against vendored 1.0.0 schemas and semantic rules. Go and Rust conformance fixtures must be generated from the same committed fixture corpus so the implementations cannot drift.

Plugin package content is not copied into database rows. Canonical records provide validation, semantic hashing, diagnostics, and deterministic applicable paths. Runtime reads the checked-out package and revalidates it.

Factory semantic hashing includes manifest, standard component configuration, skills, and package-contained files reachable by those components. A plugin-only change therefore creates a new desired source state even when projected database fields do not change.
### 7. Factory runtime scoping
Add a runtime `FactoryFileScope` snapshot with:
- Factory UID and checked-out Factory root.
- Bound agent name.
- Optional automation name.
- Ordered applicable plugin collection paths.
- Ordered applicable Factory MCP file paths.

Factory plugin collection paths are:
- Automation `plugins/`, when present.
- Bound agent `plugins/`.
- Factory `plugins/`.

The worker converts each discovered package under those collection paths into a repeated exact `--plugin-dir <package-root>` client argument. More-specific packages with the same manifest name suppress lower scopes before launch.

Factory MCP paths are:
- Automation `mcp.json`, when present.
- Bound agent `mcp.json`.
- Factory `mcp.json`.

The worker passes them as repeated `--factory-mcp <file-path>` arguments. Use repeated arguments rather than a comma-separated environment variable so valid paths cannot be split.

Factory runtime plugins are part of the applied Factory definition and start with the run. Plugin MCP servers are not project cards that require an interactive start inside a headless worker. This follows the existing behavior of MCP already attached to a Factory agent or automation. The Factory source/apply trust boundary is responsible for this difference from an interactive repository session.

Before passing either argument:
- Verify the path remains inside the checked-out Factory root.
- Verify the runtime checkout corresponds to the applied Factory source revision.
- Require a client capability that advertises Agent Plugins 1.0.0 and Factory MCP 1.0 support.
### 8. Warp Factory MCP schema
Publish and vendor an immutable closed schema at:

```text
https://warp.dev/schemas/factory-mcp/1.0.0/schema.json
```

Its shape is:

```json
{
  "$schema": "https://warp.dev/schemas/factory-mcp/1.0.0/schema.json",
  "mcpServers": {
    "search": {
      "type": "managed",
      "warpId": "00000000-0000-0000-0000-000000000000"
    },
    "lint": {
      "type": "stdio",
      "command": "./bin/lint-server",
      "args": ["--mode", "factory"],
      "cwd": "./"
    },
    "issues": {
      "type": "streamable-http",
      "url": "https://mcp.example.com/issues"
    }
  }
}
```

Top level permits only `$schema` and `mcpServers`. Each entry is exactly one closed variant.

- `managed` permits only `type` and `warpId`.
- `stdio` uses the Agent Plugins field shape, but paths are relative to the entity directory that contains the Factory MCP file.
- `streamable-http` uses the Agent Plugins field shape.
- `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` are invalid in a Factory MCP file.
- V1 does not add Factory-specific secret interpolation. Managed MCP and existing Factory secrets remain the credential path.

Factoryfile sync is authoritative for managed entries:
1. Parse the file by fixed entity location.
2. Require the Warp Factory schema identifier.
3. Validate each `warpId` in team scope.
4. Merge managed entries at the existing entity level.
5. Project them through the existing service-account and automation paths.

The client is authoritative for ordinary entries:
1. Parse only files passed through `--factory-mcp`.
2. Require the Warp Factory schema identifier.
3. Ignore valid `managed` entries without creating an installation.
4. Load stdio and Streamable HTTP entries through `FileBasedMCPManager` with Factory provenance.
5. Resolve relative paths against the containing entity directory.

The client must not accept this schema in a plugin root. The plugin MCP parser reports an unsupported Agent Plugins schema and disables only plugin MCP. Conversely, factoryfile rejects the Agent Plugins schema at an entity-level Factory MCP path.
### 9. Legacy Factory MCP migration
Keep legacy `mcpServers` parsing in `v1alpha1.go` during the transition. Convert both old and new managed entries into the existing canonical `MCPServerEntry` before resolution.

Merge by entity and server name:
- Same name and same normalized `warpId`: one entry.
- Same name and different `warpId`: source validation error naming both files.
- Ordinary entries exist only in the new Factory MCP source and are not projected as managed MCP.

Rendering preserves the source representation. It does not rewrite a user's YAML/frontmatter into a new file during reconciliation.
Root Factory `mcp.json` migrates top-level `factory.yaml` MCP, while agent and automation `mcp.json` files migrate their matching frontmatter. Keep `agentDefaults.mcpServers` as a legacy-only source in v1. There is no new default-only Factory MCP file. Migration tooling or documentation expands those defaults into each intended agent file before the legacy field is removed.

Add a source diagnostic and telemetry counter for legacy declarations. Do not set a removal release in this change.
### 10. Feature and capability rollout
Use separate gates:
- Client Agent Plugins parser/discovery.
- Factory plugin source parsing.
- Factory MCP source parsing.
- Factory runtime argument emission.

Factory sync can validate new source before every worker can run it, but apply must reject a Factory that selects a worker/client without the required capabilities. Do not silently omit requested plugins or ordinary Factory MCP servers.

Stable rollout order:
1. Ship the capable client and worker contract.
2. Enable local user and repository discovery.
3. Enable Factory source validation.
4. Enable Factory runtime propagation.
5. Enable Factory MCP authoring and legacy diagnostics.
## Decisions
### Separate plugin and Factory MCP schemas
Options considered:
- Extend Agent Plugins `mcp.json` with `warpId`. Rejected because the standard schema is closed and an entry must match a standard transport.
- Put managed references in a Warp extension directory inside each plugin. Rejected because managed MCP is Factory configuration, not a portable plugin component.
- Define entity-level Factory `mcp.json`. Selected because location and `$schema` make ownership explicit while one Factory file can carry managed and ordinary servers.
### Reuse existing execution semantics
Options considered:
- Add package-wide enablement and content-fingerprint approval. Safer for changed stdio packages, but inconsistent with equivalent existing skill and MCP sources.
- Reuse current source semantics. Selected by product direction. The implementation preserves source provenance in existing component details and logs, not a new trust system.
### Structured identity instead of string rewriting
Options considered:
- Rewrite source skill and MCP tool names. Rejected because it mutates portable metadata and MCP wire names.
- Carry structured package/component identity and render qualified labels at boundaries. Selected because routing stays stable and source packages remain portable.
### Runtime-local Factory MCP
Options considered:
- Import ordinary Factory MCP into the managed MCP database. Rejected because local package paths are meaningful only inside a checkout and worker.
- Let the client read applicable entity files from the checkout. Selected because it preserves path context and keeps the control plane from executing local commands.
## Risks and mitigations
- Rust and Go validators can drift.
  - Keep one cross-repository fixture corpus derived from the published schemas. Run it against both implementations in CI.
- A source revision can change between Factory validation and runtime.
  - Store the applied source revision and verify the checkout before emitting runtime paths.
- Global Warp-home plugins inherit automatic MCP start.
  - Show source and resolved launch details in existing MCP details and logs. Address stricter trust only through a unified file-based MCP design.
- A skill can instruct the agent to run package-supplied code.
  - Keep this on the ordinary shell-command path. Do not add a plugin bypass around execution-profile permissions, risk classification, allowlists, denylists, or approval.
- Plugin paths can escape through symlinks or platform-specific path behavior.
  - Centralize filesystem-resolved containment and test Unix symlinks plus Windows junction/reparse behavior.
- Factory plugin data can become ephemeral on a worker backend.
  - Make a writable persistent root a dispatch precondition for plugin MCP stdio servers.
- Qualified skill names can conflict with existing parser syntax.
  - Add parser fixtures for colon-qualified plugin skills and preserve repository-qualified flat skill behavior.
- Two `mcp.json` formats can be confused.
  - Require exact locations and exact schema identifiers. Emit targeted cross-format diagnostics.
## Testing and validation
### Agent Plugins conformance
Create a committed conformance fixture suite that covers every item in Appendix A of Agent Plugins 1.0.0:
- Manifest required fields, naming, unknown fields, extensions exceptions, and unsupported schema.
- Fixed component paths, missing paths, wrong filesystem kinds, and non-recursive skills.
- Symlink/path escape failures at plugin, component, skill, command, and working-directory boundaries.
- MCP top-level and per-server failure isolation.
- Stdio executable-token, default working directory, environment overlay order, reserved variables, and single non-recursive expansion.
- Streamable HTTP URL, redirect-origin, and header validation.
- Unsupported SSE isolation.
- Component start, connection, authentication, and handshake failure isolation.

Both Rust and Go validators run the applicable shared fixtures.
### Client unit and integration tests
- `PluginWatcher` tests all four local search roots, immediate-child scanning, hot reload, channel-aware Warp paths, same-name precedence, and cross-repository ambiguity.
- `PluginManager` tests package-level shadowing and diagnostic preservation.
- `SkillManager` and `resolve_skill_spec` test qualified invocation, unique unqualified alias, flat/plugin ambiguity, and cloud multi-repository scope.
- `FileBasedMCPManager` tests provider/scope auto-start parity with existing file-based MCP.
- MCP spawn tests assert exact `argv`, environment overlay order, authoritative variables, default `cwd`, persistent data path, and native tool-name routing.
- Factory MCP client tests assert managed entries are ignored and ordinary entries load.
- Cross-format tests assert Factory schema in a plugin root and Agent Plugins schema at a Factory entity path produce the specified isolated diagnostics.

Run at minimum:

```text
cargo test -p ai plugins
cargo test -p warp --lib ai::plugins
cargo test -p warp --lib ai::skills
cargo test -p warp --lib ai::mcp
cargo test -p warp_tui plugins
cargo fmt --all -- --check
```

### Factory tests
- Parser fixtures cover all three plugin scopes and all three Factory MCP locations.
- Resolution tests cover automation > agent > factory plugin shadowing.
- Managed MCP tests cover legacy/new deduplication, conflicts, scope, team validation, and projection.
- Dispatch tests cover ordered exact paths, checkout containment, source revision, capability rejection, and persistent-data requirements.
- End-to-end tests run a factory skill, a plugin stdio MCP tool, an ordinary entity-level MCP tool, and a projected managed MCP from one Factory.
- Run the same end-to-end case on a Warp-hosted sandbox and a self-hosted Docker worker. Add a direct-worker contract test for its durable data root.

Run at minimum in `warp-server`:

```text
go test ./logic/factoryfile/...
go test ./logic/ai/ambient_agents/workers/common/...
go test ./logic/ai/ambient_agents/... -run 'Factory|Plugin|MCP'
gofmt -l logic/factoryfile logic/ai/ambient_agents
```

### User-visible proof
- Record a desktop video that adds a repository plugin, shows its qualified skill in the existing skill list, invokes it, shows the qualified MCP server in existing MCP settings, explicitly starts the project server, and uses one tool.
- Record a TUI video that shows the qualified skill and MCP server in the existing component surfaces, invokes the skill, starts the server, and uses one tool.
- Record a Factory run artifact that identifies the active factory/agent/automation plugin scopes and successfully calls both an ordinary plugin MCP tool and a managed Factory MCP tool.
## Parallelization
Implementation can use two workstreams after the shared identities and JSON contracts are agreed:
- Client: Rust schemas, parser, watcher, SkillManager, MCP manager, data locator, and minimal existing-surface adapters on a Warp worktree.
- Factory: Go source parsing, Factory MCP schema, projection, runtime scope, and worker contract on a warp-server worktree.

Factory development can proceed in parallel against committed fixture/schema contracts, but end-to-end Factory rollout waits for the client capability. Use one PR per repository; keep the PRODUCT and TECH specs aligned in this Warp PR.
## Assumptions
- The Factory source revision can be recorded and compared with the worker checkout before launch.
- Each supported worker backend can expose a durable writable plugin-data root. If this is false for a backend, that backend cannot claim plugin MCP stdio conformance until storage exists.
- Existing Factory source registration and apply permissions remain the product trust boundary for repository code.
- The implementation can publish the proposed immutable Warp Factory MCP schema URL before enabling authoring.
## Out of scope
- Claude Code conversion or provider implementation.
- A dedicated GUI or TUI plugin inventory and plugin-level management controls.
- Plugin distribution and installation.
- New secret-reference fields in either plugin or Factory ordinary MCP entries.
- A generalized permissions or subprocess-sandbox redesign.
- Automatic legacy YAML rewriting or removal.
- Agent Plugins legacy SSE transport.
