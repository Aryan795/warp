//! Grammar selection for the tree-sitter shell parser adapter (APP-5430).
//!
//! This module is intentionally minimal for Phase 0 ("corpus and grammar readiness"): it owns
//! Arborium grammar selection per dialect, which is the prerequisite for writing grammar
//! conformance tests. The Warp-owned parse model (`ParsedShellInput`, `ParsedCommand`, EOF
//! recovery, etc.) described in `specs/APP-5430/TECH.md` is built in Phase 1 on top of this.
//!
//! No item exported from `warp_completer`'s public API may leak an Arborium or tree-sitter type;
//! see `no_backend_types_in_public_api` below. This module itself uses `tree_sitter::Language`
//! internally and stays private to the crate until Phase 1 defines the Warp-owned model that
//! wraps it.

use arborium::tree_sitter::Language;

/// The shell dialects the adapter must support.
///
/// The spec's original grammar viability gate prohibited mapping Zsh to the Bash grammar as an
/// approximation, because Arborium 2.18.1's own Zsh grammar (`georgeharker/tree-sitter-zsh`)
/// fails to parse even the simplest complete command without error (see the grammar conformance
/// tests). Fixing or replacing that grammar is an open-ended upstream investigation, not a
/// narrowly scoped patch. The requester decided to lift that prohibition for Phase 0: `Zsh` maps
/// to the Bash grammar below. Zsh-only syntax the Bash grammar cannot parse (e.g. `=(...)`
/// process substitution, anonymous functions, short-form loops) is expected to produce `ERROR`
/// nodes or an incorrect hierarchy; those divergences are captured explicitly in
/// `grammar_tests.rs` rather than assumed away.
///
/// Only grammar conformance tests construct this in Phase 0; the Phase 1 adapter is the first
/// production consumer, so `#[allow(dead_code)]` avoids a premature lint failure on scaffolding
/// that is intentionally unused outside tests for now.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellDialect {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

#[allow(dead_code)]
impl ShellDialect {
    /// Returns the Arborium language name used by `arborium::get_language`.
    fn arborium_name(self) -> &'static str {
        match self {
            // Zsh intentionally reuses the Bash grammar; see the `ShellDialect` doc comment.
            Self::Bash | Self::Zsh => "bash",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }

    /// Returns the tree-sitter grammar for this dialect.
    ///
    /// Panics if the corresponding Arborium feature is disabled, which would be a build
    /// configuration bug: all three `lang-bash`/`lang-fish`/`lang-powershell` features are
    /// unconditionally enabled on the `arborium` dependency.
    pub(crate) fn grammar(self) -> Language {
        arborium::get_language(self.arborium_name())
            .unwrap_or_else(|| panic!("arborium grammar for {self:?} must be enabled"))
    }
}

#[cfg(test)]
#[path = "grammar_tests.rs"]
mod grammar_tests;
