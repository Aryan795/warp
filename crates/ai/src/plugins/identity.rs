//! Structured identity for plugin packages and the components they provide.
//!
//! Identity is structured rather than a display string so routing stays stable while UI and
//! model adapters render the qualified `<plugin>:<component>` label at their boundaries. Source
//! identity is deliberately opaque: adding a remote source kind later must not change the
//! identity of a component that a conversation already referenced.
use std::fmt;

use serde::{Deserialize, Serialize};

/// Separator between a plugin name and a component name in a qualified component name.
pub const QUALIFIED_NAME_SEPARATOR: char = ':';

/// Where a plugin package was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginSourceKind {
    /// A `.agents/plugins` directory, in the user's home directory or a repository.
    AgentsDirectory,
    /// A Warp config `plugins` directory, in the Warp home config directory or a repository's
    /// `.warp` directory.
    WarpDirectory,
    /// A plugin collection inside a checked-out Factory source repository.
    FactoryRepository,
}

impl PluginSourceKind {
    /// Rank used to break ties between two sources at the same scope. Lower wins.
    ///
    /// This mirrors the `.agents`-before-`.warp` order in the flat skill provider list.
    pub fn provider_rank(self) -> u8 {
        match self {
            PluginSourceKind::AgentsDirectory => 0,
            PluginSourceKind::WarpDirectory => 1,
            PluginSourceKind::FactoryRepository => 2,
        }
    }
}

/// Identifies the provider directory a package came from, independently of its version.
///
/// `stable_identity` names the user root, repository, or Factory source. It is opaque at
/// component boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginSourceId {
    pub kind: PluginSourceKind,
    pub stable_identity: String,
}

impl PluginSourceId {
    pub fn new(kind: PluginSourceKind, stable_identity: impl Into<String>) -> Self {
        Self {
            kind,
            stable_identity: stable_identity.into(),
        }
    }
}

/// The scope a plugin instance belongs to.
///
/// Scope separates otherwise identically named packages so their runtime state — most
/// importantly their persistent data directory — never collides.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginScopeId {
    /// A plugin discovered under one of the user's home search roots.
    User,
    /// A plugin discovered under a repository search root.
    Repository,
    /// A factory-scoped Factory plugin.
    Factory,
    /// An agent-scoped Factory plugin.
    Agent { name: String },
    /// An automation-scoped Factory plugin.
    Automation { name: String },
}

impl PluginScopeId {
    /// Rank used to order two candidates for the same plugin name. Lower wins.
    ///
    /// Repository scope outranks user scope for interactive sessions, and the Factory scopes
    /// order automation over agent over factory.
    pub fn scope_rank(&self) -> u8 {
        match self {
            PluginScopeId::Automation { .. } => 0,
            PluginScopeId::Agent { .. } => 1,
            PluginScopeId::Factory => 2,
            PluginScopeId::Repository => 3,
            PluginScopeId::User => 4,
        }
    }

    /// A short stable token for this scope, used when deriving persistent data keys.
    pub fn key_token(&self) -> String {
        match self {
            PluginScopeId::User => "user".to_owned(),
            PluginScopeId::Repository => "repository".to_owned(),
            PluginScopeId::Factory => "factory".to_owned(),
            PluginScopeId::Agent { name } => format!("agent/{name}"),
            PluginScopeId::Automation { name } => format!("automation/{name}"),
        }
    }
}

impl fmt::Display for PluginScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginScopeId::User => write!(f, "user"),
            PluginScopeId::Repository => write!(f, "repository"),
            PluginScopeId::Factory => write!(f, "factory"),
            PluginScopeId::Agent { name } => write!(f, "agent {name}"),
            PluginScopeId::Automation { name } => write!(f, "automation {name}"),
        }
    }
}

/// Identifies one loaded plugin instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginInstanceId {
    pub scope: PluginScopeId,
    pub source: PluginSourceId,
    pub manifest_name: String,
}

impl PluginInstanceId {
    pub fn new(
        scope: PluginScopeId,
        source: PluginSourceId,
        manifest_name: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            source,
            manifest_name: manifest_name.into(),
        }
    }

    /// Precedence tuple for shadowing: `(scope rank, provider rank)`. Lower wins.
    pub fn precedence(&self) -> (u8, u8) {
        (self.scope.scope_rank(), self.source.kind.provider_rank())
    }
}

/// The standard component types Agent Plugins 1.0.0 defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginComponentKind {
    Skill,
    McpServer,
}

impl fmt::Display for PluginComponentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginComponentKind::Skill => write!(f, "skill"),
            PluginComponentKind::McpServer => write!(f, "MCP server"),
        }
    }
}

/// Identifies one component provided by one plugin instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginComponentId {
    pub plugin: PluginInstanceId,
    pub kind: PluginComponentKind,
    pub local_name: String,
}

impl PluginComponentId {
    pub fn new(
        plugin: PluginInstanceId,
        kind: PluginComponentKind,
        local_name: impl Into<String>,
    ) -> Self {
        Self {
            plugin,
            kind,
            local_name: local_name.into(),
        }
    }

    /// The `<plugin>:<component>` name shown to users and sent to the model.
    ///
    /// This is runtime identity metadata. The component's own portable metadata — a skill's
    /// frontmatter `name`, an MCP server's native tool names — is never rewritten.
    pub fn qualified_name(&self) -> String {
        format!(
            "{}{QUALIFIED_NAME_SEPARATOR}{}",
            self.plugin.manifest_name, self.local_name
        )
    }
}

impl fmt::Display for PluginComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.qualified_name())
    }
}

/// Splits a possibly qualified component name into its plugin and component parts.
///
/// Returns `None` when `name` has no separator, which means the caller must resolve it as an
/// unqualified name. A plugin name can itself contain periods but never a colon, so the first
/// separator is the boundary.
pub fn split_qualified_name(name: &str) -> Option<(&str, &str)> {
    let (plugin, component) = name.split_once(QUALIFIED_NAME_SEPARATOR)?;
    if plugin.is_empty() || component.is_empty() {
        return None;
    }
    Some((plugin, component))
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
