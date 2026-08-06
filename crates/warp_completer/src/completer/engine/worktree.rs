//! Worktree-aware completion suggestions.
//!
//! Git worktree directories are frequently short or auto-generated, and they
//! usually live outside the current working directory, so ordinary filesystem
//! path completion never surfaces them. This module surfaces a repository's
//! known worktrees as completion suggestions in path-like command contexts
//! (e.g. `cd`, `git worktree remove`), matching on the worktree's memorable
//! name while inserting its absolute path so it resolves from anywhere.

use warp_command_signatures::IconType;
use warp_util::path::ShellFamily;

use crate::completer::engine::path::EngineFileType;
use crate::completer::matchers::MatchStrategy;
use crate::completer::suggest::{MatchedSuggestion, Priority, Suggestion, SuggestionType};
use crate::parsers::ParsedToken;

/// A git worktree known to the current repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    /// A memorable name for the worktree — its checked-out branch name when
    /// present, otherwise the working directory's basename.
    pub name: String,
    /// Absolute path to the worktree's working directory.
    pub path: String,
}

/// Git `worktree` subcommands whose argument references an existing worktree by
/// path, and for which worktree-name suggestions are therefore useful.
const WORKTREE_SUBCOMMANDS: &[&str] = &["remove", "move", "lock", "unlock", "repair"];

/// Returns whether the command being completed is one where worktree names are
/// useful suggestions: `cd`, or a `git worktree` subcommand that references an
/// existing worktree.
///
/// `tokens_without_last_editing` are the command's tokens excluding the token
/// currently being edited (e.g. `["cd"]` while typing `cd my-wo`).
pub fn is_worktree_completion_context(tokens_without_last_editing: &[&str]) -> bool {
    match tokens_without_last_editing {
        [command, ..] if *command == "cd" => true,
        ["git", "worktree", subcommand, ..] => WORKTREE_SUBCOMMANDS.contains(subcommand),
        _ => false,
    }
}

/// Builds worktree-name suggestions for `token`, keeping only worktrees whose
/// name matches the partially-typed token under `matcher`. Each suggestion
/// displays the worktree's name but inserts its shell-escaped absolute path, so
/// accepting one resolves correctly regardless of the current directory.
pub(crate) fn worktree_suggestions(
    token: &ParsedToken,
    matcher: MatchStrategy,
    worktrees: &[Worktree],
    shell_family: ShellFamily,
) -> Vec<MatchedSuggestion> {
    worktrees
        .iter()
        .filter_map(|worktree| {
            let match_type = matcher.get_match_type(token.as_str(), worktree.name.as_str())?;
            let replacement = shell_family.shell_escape(&worktree.path).into_owned();
            let mut suggestion = Suggestion::new(
                worktree.name.as_str(),
                replacement,
                Some(format!("Worktree · {}", worktree.path)),
                SuggestionType::Argument,
                Priority::default(),
            );
            suggestion.file_type = Some(EngineFileType::Directory);
            suggestion.override_icon = Some(IconType::Folder);
            Some(MatchedSuggestion::new(suggestion, match_type))
        })
        .collect()
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
