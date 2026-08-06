use warp_util::path::ShellFamily;

use super::{Worktree, is_worktree_completion_context, worktree_suggestions};
use crate::completer::matchers::MatchStrategy;
use crate::parsers::ParsedToken;

fn worktrees() -> Vec<Worktree> {
    vec![
        Worktree {
            name: "canyon".to_owned(),
            path: "/Users/me/worktrees/canyon".to_owned(),
        },
        Worktree {
            name: "mesa".to_owned(),
            path: "/Users/me/worktrees/mesa".to_owned(),
        },
        Worktree {
            name: "main".to_owned(),
            path: "/Users/me/project".to_owned(),
        },
    ]
}

#[test]
fn is_worktree_context_for_cd() {
    assert!(is_worktree_completion_context(&["cd"]));
}

#[test]
fn is_worktree_context_for_git_worktree_subcommands() {
    assert!(is_worktree_completion_context(&[
        "git", "worktree", "remove"
    ]));
    assert!(is_worktree_completion_context(&["git", "worktree", "move"]));
    assert!(is_worktree_completion_context(&["git", "worktree", "lock"]));
    assert!(is_worktree_completion_context(&[
        "git", "worktree", "unlock"
    ]));
    assert!(is_worktree_completion_context(&[
        "git", "worktree", "repair"
    ]));
}

#[test]
fn is_not_worktree_context_for_unrelated_commands() {
    assert!(!is_worktree_completion_context(&["ls"]));
    assert!(!is_worktree_completion_context(&["git", "commit"]));
    // `git worktree add` creates a new worktree, so existing worktrees are not
    // useful completions there.
    assert!(!is_worktree_completion_context(&["git", "worktree", "add"]));
    assert!(!is_worktree_completion_context(&[]));
}

#[test]
fn suggests_worktrees_matching_the_typed_prefix() {
    let suggestions = worktree_suggestions(
        &ParsedToken::new("ca"),
        MatchStrategy::CaseInsensitive,
        &worktrees(),
        ShellFamily::Posix,
    );

    let displays: Vec<&str> = suggestions.iter().map(|s| s.display()).collect();
    assert_eq!(displays, vec!["canyon"]);
}

#[test]
fn accepting_a_suggestion_inserts_the_worktree_path() {
    let suggestions = worktree_suggestions(
        &ParsedToken::new("mesa"),
        MatchStrategy::CaseInsensitive,
        &worktrees(),
        ShellFamily::Posix,
    );

    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].replacement(), "/Users/me/worktrees/mesa");
}

#[test]
fn shell_escapes_worktree_paths_with_special_characters() {
    let worktrees = vec![Worktree {
        name: "feature".to_owned(),
        path: "/Users/me/work trees/feature".to_owned(),
    }];

    let suggestions = worktree_suggestions(
        &ParsedToken::new("fea"),
        MatchStrategy::CaseInsensitive,
        &worktrees,
        ShellFamily::Posix,
    );

    assert_eq!(suggestions.len(), 1);
    assert_eq!(
        suggestions[0].replacement(),
        r"/Users/me/work\ trees/feature"
    );
}

#[test]
fn produces_no_suggestions_when_nothing_matches() {
    let suggestions = worktree_suggestions(
        &ParsedToken::new("zzz"),
        MatchStrategy::CaseInsensitive,
        &worktrees(),
        ShellFamily::Posix,
    );

    assert!(suggestions.is_empty());
}

#[test]
fn empty_token_matches_all_worktrees() {
    let suggestions = worktree_suggestions(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &worktrees(),
        ShellFamily::Posix,
    );

    assert_eq!(suggestions.len(), 3);
}
