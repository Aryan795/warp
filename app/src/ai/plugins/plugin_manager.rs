//! The client-side owner of discovered Agent Plugin packages.
//!
//! `PluginManager` scans the fixed plugin search roots, keeps the winning package for each
//! manifest name, and turns the `Agent Plugin discovery` preference into the teardown the rest
//! of the client has to perform. It never launches anything itself: a discovered stdio server
//! becomes an installation for the existing file-based MCP surfaces, which own starting it.
use std::collections::BTreeSet;
use std::path::PathBuf;

use ai::plugins::{
    LocalPluginDataLocator, PluginCandidate, PluginComponentId, PluginDiagnostic, PluginFrontend,
    PluginSkillComponent, repository_search_roots, resolve_active_packages, scan_search_root,
    user_search_roots,
};
use repo_metadata::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::registry::{PluginDiscoveryPolicy, PluginRegistry, PluginTeardownStep};
use crate::settings::AISettingsChangedEvent;
use crate::settings::ai::AISettings;

/// What the plugin manager tells the rest of the client.
pub enum PluginManagerEvent {
    /// The active plugin set changed. Skill and MCP surfaces re-read it.
    PluginsChanged,
    /// Plugin skills must leave the model catalog and the explicit invocation resolver.
    WithdrawSkills,
    /// In-flight plugin MCP tool calls must be cancelled with `agent_plugin_discovery_disabled`.
    CancelInFlightMcpCalls,
    /// These plugin MCP installations must be stopped and unregistered.
    UnregisterMcpInstallations { components: Vec<PluginComponentId> },
}

pub struct PluginManager {
    registry: PluginRegistry,
    policy: PluginDiscoveryPolicy,
    /// Repository roots currently in scope. Plugins from a repository are only active while its
    /// repository is, matching the existing skill scoping rules.
    repository_roots: BTreeSet<PathBuf>,
    data_locator: LocalPluginDataLocator,
}

impl PluginManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let policy = PluginDiscoveryPolicy::InteractivePreference;
        let enabled = FeatureFlag::AgentPlugins.is_enabled()
            && policy.is_enabled(AISettings::as_ref(ctx).is_plugin_discovery_enabled(ctx));

        if FeatureFlag::AgentPlugins.is_enabled() {
            ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
                if matches!(
                    event,
                    AISettingsChangedEvent::AgentPluginDiscoveryEnabled { .. }
                ) {
                    me.handle_discovery_preference_change(ctx);
                }
            });
            ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, _, event, ctx| {
                let DetectedRepositoriesEvent::DetectedGitRepo { repository, .. } = event;
                let root = repository.as_ref(ctx).root_dir().to_local_path_lossy();
                if me.repository_roots.insert(root) {
                    me.rescan(ctx);
                }
            });
        }

        let mut manager = Self {
            registry: PluginRegistry::new(enabled),
            policy,
            repository_roots: BTreeSet::new(),
            data_locator: LocalPluginDataLocator::new(
                warp_core::paths::data_dir(),
                active_frontend(),
            ),
        };
        if enabled {
            manager.rescan(ctx);
        }
        manager
    }

    /// The persistent data directory for a plugin instance, without creating it.
    ///
    /// The directory is created immediately before a stdio server's first start, never during
    /// discovery, so validating a package can never allocate storage for it.
    pub fn data_locator(&self) -> &LocalPluginDataLocator {
        &self.data_locator
    }

    pub fn is_discovery_enabled(&self) -> bool {
        self.registry.is_enabled()
    }

    pub fn active_skills(&self) -> Vec<&PluginSkillComponent> {
        self.registry.active_skills()
    }

    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        self.registry.diagnostics()
    }

    /// Resolves an explicit skill reference that may be plugin-qualified.
    pub fn resolve_skill(
        &self,
        name: &str,
        flat_names: &[String],
    ) -> Result<&PluginSkillComponent, PluginDiagnostic> {
        self.registry.resolve_skill(name, flat_names)
    }

    /// Applies a change to the `Agent Plugin discovery` preference.
    ///
    /// The registry has already stopped answering lookups by the time the teardown events are
    /// emitted, so a turn that starts mid-teardown cannot resolve a component that is on its way
    /// out.
    fn handle_discovery_preference_change(&mut self, ctx: &mut ModelContext<Self>) {
        let enabled = self
            .policy
            .is_enabled(AISettings::as_ref(ctx).is_plugin_discovery_enabled(ctx));
        let transition = self.registry.set_enabled(enabled);
        if transition.is_noop() {
            return;
        }

        for step in transition.teardown {
            if let Some(event) = teardown_event(step) {
                ctx.emit(event);
            }
        }

        if transition.rescan {
            self.rescan(ctx);
        } else {
            // Package files and plugin data are left on disk; only the runtime set is empty.
            ctx.emit(PluginManagerEvent::PluginsChanged);
        }
    }

    /// Rebuilds the active plugin set from every in-scope search root.
    ///
    /// Scanning reads `plugin.json`, `skills/`, and `mcp.json` and nothing else. A generation tag
    /// makes the result droppable, so a scan that finishes after discovery was turned off cannot
    /// resurrect the packages the teardown just removed.
    fn rescan(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.registry.is_enabled() {
            return;
        }
        let generation = self.registry.begin_scan();

        let mut candidates: Vec<PluginCandidate> = Vec::new();
        for root in user_search_roots() {
            candidates.extend(scan_search_root(&root));
        }
        for repo_root in &self.repository_roots {
            for root in repository_search_roots(repo_root) {
                candidates.extend(scan_search_root(&root));
            }
        }

        let resolved = resolve_active_packages(candidates);
        for diagnostic in resolved.all_diagnostics() {
            log_plugin_diagnostic(&diagnostic);
        }
        if self.registry.apply_scan(generation, resolved) {
            ctx.emit(PluginManagerEvent::PluginsChanged);
        }
    }
}

/// Emits a package-level diagnostic to structured logs.
///
/// Component-level status continues to reach the user through the existing Skills and MCP
/// surfaces; this is the channel for problems that leave no component behind to attach to.
fn log_plugin_diagnostic(diagnostic: &PluginDiagnostic) {
    if diagnostic.is_error() {
        log::warn!("{diagnostic}");
    } else {
        log::info!("{diagnostic}");
    }
}

fn active_frontend() -> PluginFrontend {
    match settings::settings_mode() {
        settings::SettingsMode::Tui => PluginFrontend::Tui,
        _ => PluginFrontend::Gui,
    }
}

impl Entity for PluginManager {
    type Event = PluginManagerEvent;
}

impl SingletonEntity for PluginManager {}

/// Turns a teardown step into the event the rest of the client acts on.
///
/// `StopWatchers` has no event: the manager owns its own scanning, and stopping it is the
/// generation bump the registry already performed.
pub(crate) fn teardown_event(step: PluginTeardownStep) -> Option<PluginManagerEvent> {
    match step {
        PluginTeardownStep::StopWatchers => None,
        PluginTeardownStep::WithdrawSkills => Some(PluginManagerEvent::WithdrawSkills),
        PluginTeardownStep::CancelInFlightMcpCalls => {
            Some(PluginManagerEvent::CancelInFlightMcpCalls)
        }
        PluginTeardownStep::UnregisterMcpInstallations { components } => {
            Some(PluginManagerEvent::UnregisterMcpInstallations { components })
        }
    }
}
