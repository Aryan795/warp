//! Phase 1 adapter tests (`specs/APP-5430/TECH.md`), exercising the public `parse_shell_input`
//! API rather than raw tree-sitter output (see `grammar_tests.rs` for that). Module names match
//! the spec's `cargo test -p warp_completer <name>` invocations so each can be run in isolation.

use string_offset::ByteOffset;

use super::{
    ParsedCommand, ShellDialect, ShellParseOptions, ShellParseRejection, ShellParseStatus,
    parse_shell_input,
};

fn parse(dialect: ShellDialect, source: &str) -> super::ParsedShellInput {
    parse_shell_input(source, dialect, ShellParseOptions::default())
}

fn executable(command: &ParsedCommand) -> Option<&str> {
    command.executable.as_ref().map(|e| e.item.as_str())
}

mod shell_adapter_complete_corpus {
    use super::*;

    #[test]
    fn dollar_paren_nesting_three_levels() {
        let source = "echo \"$(a $(b $(c)))\"";
        let result = parse(ShellDialect::Bash, source);
        assert_eq!(result.status, ShellParseStatus::Complete);
        let outer = &result.commands[0];
        assert_eq!(executable(outer), Some("echo"));
        let level1 = &outer.nested_groups[0];
        assert_eq!(
            &source[level1.content_span.start()..level1.content_span.end()],
            "a $(b $(c))"
        );
        let level2 = &level1.commands[0].nested_groups[0];
        assert_eq!(
            &source[level2.content_span.start()..level2.content_span.end()],
            "b $(c)"
        );
        let level3 = &level2.commands[0].nested_groups[0];
        assert_eq!(
            &source[level3.content_span.start()..level3.content_span.end()],
            "c"
        );
        assert!(level3.commands[0].nested_groups.is_empty());
    }

    #[test]
    fn process_substitution_is_a_nested_group_not_a_second_top_level_command() {
        let source = "cat <(printf x)";
        let result = parse(ShellDialect::Bash, source);
        assert_eq!(result.status, ShellParseStatus::Complete);
        assert_eq!(
            result.commands.len(),
            1,
            "printf x must not be a separate top-level command"
        );
        assert_eq!(executable(&result.commands[0]), Some("cat"));
        let group = &result.commands[0].nested_groups[0];
        assert_eq!(
            group.kind,
            super::super::NestedCommandKind::InputProcessSubstitution
        );
        assert_eq!(executable(&group.commands[0]), Some("printf"));
    }

    #[test]
    fn assignment_and_redirect_inside_nested_command() {
        let source = "echo \"$(KEY=VALUE env >out)\"";
        let result = parse(ShellDialect::Bash, source);
        assert_eq!(result.status, ShellParseStatus::Complete);
        let nested = &result.commands[0].nested_groups[0].commands[0];
        assert_eq!(executable(nested), Some("env"));
        assert_eq!(nested.leading_assignments.len(), 1);
        assert_eq!(nested.leading_assignments[0].item, "KEY=VALUE");
        assert_eq!(nested.redirections.len(), 1);
        let redirect = &nested.redirections[0];
        assert_eq!(redirect.kind, super::super::ShellRedirectionKind::Output);
        let destination = redirect.destination_span.unwrap();
        assert_eq!(&source[destination.start()..destination.end()], "out");
    }
}

mod shell_adapter_incomplete_corpus {
    use super::*;

    #[test]
    fn unclosed_dollar_paren_recovers_open() {
        let source = "echo pre$(pw";
        let result = parse(ShellDialect::Bash, source);
        let ShellParseStatus::Recovered { open_delimiters } = &result.status else {
            panic!("expected Recovered, got {:?}", result.status);
        };
        assert_eq!(open_delimiters, &[super::super::OpenDelimiter::DollarParen]);
        let group = &result.commands[0].nested_groups[0];
        assert_eq!(group.closure, super::super::DelimiterState::Open);
        assert_eq!(executable(&group.commands[0]), Some("pw"));
        // No synthetic bytes leak into any returned text.
        for command in result.commands_depth_first() {
            if let Some(exec) = &command.executable {
                assert!(source.contains(exec.item.as_str()));
            }
        }
    }

    #[test]
    fn deep_incomplete_input_selects_innermost_open_group() {
        let source = "echo \"$(a $(b $(c";
        let result = parse(ShellDialect::Bash, source);
        assert!(matches!(result.status, ShellParseStatus::Recovered { .. }));
        let cursor = result.completion_command_at(ByteOffset::from(source.len()));
        assert_eq!(executable(cursor.unwrap()), Some("c"));
        let deepest = result.deepest_command_at(ByteOffset::from(source.len()));
        assert_eq!(executable(deepest.unwrap()), Some("c"));
    }

    #[test]
    fn fish_requires_a_trailing_newline_and_the_adapter_supplies_one() {
        let source = "echo (pwd)";
        let result = parse(ShellDialect::Fish, source);
        assert_eq!(result.status, ShellParseStatus::Complete);
        assert_eq!(
            result.source_len,
            source.len(),
            "the sentinel must not appear in source_len"
        );
        let group = &result.commands[0].nested_groups[0];
        assert_eq!(group.closure, super::super::DelimiterState::Closed);
        assert!(
            group.span.end() <= source.len(),
            "the sentinel newline must be clipped from every span"
        );
    }
}

/// Renumbered per the spec correction: seven confirmed failures (the original item 5, "deep
/// incomplete input", did not reproduce and is a known-good control instead -- see
/// `shell_adapter_known_good_parity`).
mod shell_adapter_legacy_failure_corpus {
    use super::*;

    #[test]
    fn nested_command_in_unquoted_concatenated_word() {
        let source = "echo pre$(pwd)post";
        let result = parse(ShellDialect::Bash, source);
        let deepest = result.deepest_command_at(ByteOffset::from(11)).unwrap();
        assert_eq!(executable(deepest), Some("pwd"));
    }

    #[test]
    fn open_nested_command_in_unquoted_concatenated_word() {
        let source = "echo pre$(pw";
        let result = parse(ShellDialect::Bash, source);
        let cursor = result
            .deepest_command_at(ByteOffset::from(source.len()))
            .unwrap();
        assert_eq!(executable(cursor), Some("pw"));
        let completion = result
            .completion_command_at(ByteOffset::from(source.len()))
            .unwrap();
        assert_eq!(executable(completion), Some("pw"));
        assert!(matches!(result.status, ShellParseStatus::Recovered { .. }));
    }

    #[test]
    fn nested_command_in_quoted_concatenated_word() {
        let source = "echo \"pre$(pwd)post\"";
        let result = parse(ShellDialect::Bash, source);
        let deepest = result.deepest_command_at(ByteOffset::from(12)).unwrap();
        assert_eq!(executable(deepest), Some("pwd"));
    }

    #[test]
    fn nested_depth_inside_adjacent_text() {
        let source = "echo \"pre$(a $(b))post\"";
        let result = parse(ShellDialect::Bash, source);
        let deepest = result.deepest_command_at(ByteOffset::from(15)).unwrap();
        assert_eq!(executable(deepest), Some("b"));
    }

    #[test]
    fn process_substitution_keeps_cat_top_level_and_printf_nested() {
        let source = "cat <(printf x)";
        let result = parse(ShellDialect::Bash, source);
        assert_eq!(result.commands.len(), 1);
        let deepest = result.deepest_command_at(ByteOffset::from(9)).unwrap();
        assert_eq!(executable(deepest), Some("printf"));
    }

    #[test]
    fn redirect_inside_nested_command_is_not_a_positional_argument() {
        let source = "echo \"$(KEY=VALUE env >out)\"";
        let result = parse(ShellDialect::Bash, source);
        let nested = &result.commands[0].nested_groups[0].commands[0];
        assert_eq!(executable(nested), Some("env"));
        assert_eq!(nested.leading_assignments.len(), 1);
        assert_eq!(nested.redirections.len(), 1);
        // "out" must appear only as the redirect destination, not as a positional word.
        assert!(!nested.parts.iter().any(|w| w.completion_value == "out"));
    }

    #[test]
    fn escaped_nested_backticks_expose_inner_command_app_5433() {
        let source = "echo `echo \\`rm -rf /\\``";
        let result = parse(ShellDialect::Bash, source);
        let decomposed = result.decompose_for_permissions(source);
        assert!(
            decomposed.commands.contains(&"rm -rf /".to_string()),
            "expected the innermost `rm -rf /` to be exposed, got {:?}",
            decomposed.commands
        );
    }
}

mod shell_adapter_known_good_parity {
    use super::*;

    #[test]
    fn simple_quoted_nested_command_cursor() {
        let source = "echo \"$(pwd)\"";
        let result = parse(ShellDialect::Bash, source);
        let deepest = result.deepest_command_at(ByteOffset::from(9)).unwrap();
        assert_eq!(executable(deepest), Some("pwd"));
    }

    #[test]
    fn simple_quoted_open_nested_command() {
        let source = "echo \"$(pw";
        let result = parse(ShellDialect::Bash, source);
        let completion = result
            .completion_command_at(ByteOffset::from(source.len()))
            .unwrap();
        assert_eq!(executable(completion), Some("pw"));
    }

    /// The spec's evidence corpus originally listed this as failure item 5; it does not
    /// reproduce (`command_at_cursor_position`/`deepest_command_at` already recurse correctly),
    /// so the spec now lists it as a known-good control and this test protects it from regressing.
    #[test]
    fn deep_incomplete_input_already_recurses_correctly() {
        let source = "echo \"$(a $(b $(c";
        let result = parse(ShellDialect::Bash, source);
        let deepest = result
            .deepest_command_at(ByteOffset::from(source.len()))
            .unwrap();
        assert_eq!(executable(deepest), Some("c"));
    }

    #[test]
    fn unescaped_nesting_exposes_inner_command() {
        for source in ["echo $(echo `rm -rf /`)", "echo `echo $(rm -rf /)`"] {
            let result = parse(ShellDialect::Bash, source);
            let decomposed = result.decompose_for_permissions(source);
            assert!(
                decomposed.commands.contains(&"rm -rf /".to_string()),
                "expected {source:?} to expose `rm -rf /`, got {:?}",
                decomposed.commands
            );
        }
    }

    #[test]
    fn single_quoted_substitution_is_literal() {
        let source = "echo '$(pwd)'";
        let result = parse(ShellDialect::Bash, source);
        assert!(result.commands[0].nested_groups.is_empty());
        let decomposed = result.decompose_for_permissions(source);
        assert_eq!(decomposed.commands, vec![source.to_string()]);
    }

    #[test]
    fn pipeline_inside_substitution_decomposes() {
        let source = "ls $(foo | echo)";
        let result = parse(ShellDialect::Bash, source);
        let decomposed = result.decompose_for_permissions(source);
        for expected in ["foo", "echo", "foo | echo", "ls $(foo | echo)"] {
            assert!(
                decomposed.commands.contains(&expected.to_string()),
                "expected {expected:?} in {:?}",
                decomposed.commands
            );
        }
    }
}

/// Covers the escaped-backtick context/parity model in `mapper::detect_escaped_backtick_group`,
/// added after a review round caught two bugs in an earlier version: (1) it fired on a literal
/// escaped backtick with no enclosing real backtick substitution, fabricating a command that
/// does not exist, and (2) EOF recovery lost an unclosed escaped nested backtick, exposing no
/// inner command at all. Both cases are covered here alongside the corpus's original closed case.
mod shell_adapter_escaped_backtick_context_and_parity {
    use super::*;

    /// A top-level escaped backtick pair with no enclosing real backtick substitution is literal
    /// text in Bash (it just prints the backslash-backtick characters); it must not be treated as
    /// a nested command.
    #[test]
    fn literal_escaped_backtick_outside_any_backtick_substitution_is_not_nested() {
        let source = r"echo \`rm -rf /\`";
        let result = parse(ShellDialect::Bash, source);
        assert_eq!(result.status, ShellParseStatus::Complete);
        assert!(
            result.commands[0].nested_groups.is_empty(),
            "a literal escaped backtick outside backtick-substitution context must not become a nested group"
        );
        let decomposed = result.decompose_for_permissions(source);
        assert_eq!(
            decomposed.commands,
            vec![source.to_string()],
            "must not fabricate a `rm -rf /` command that Bash would not execute as one"
        );
    }

    /// An escaped backtick opened but never closed, inside an outer real backtick substitution
    /// that is *also* unclosed, must still expose the inner command as an open group -- the
    /// deepest-open-command contract that incomplete-input recovery relies on.
    #[test]
    fn unclosed_escaped_backtick_inside_unclosed_outer_backtick_exposes_inner_command() {
        let source = r"echo `echo \`rm";
        let result = parse(ShellDialect::Bash, source);
        assert!(matches!(result.status, ShellParseStatus::Recovered { .. }));
        let decomposed = result.decompose_for_permissions(source);
        assert!(
            decomposed.commands.contains(&"rm".to_string()),
            "expected the unclosed inner `rm` to be exposed, got {:?}",
            decomposed.commands
        );
        let cursor = result
            .completion_command_at(ByteOffset::from(source.len()))
            .unwrap();
        assert_eq!(executable(cursor), Some("rm"));
    }

    /// The outer real backtick can be unclosed while the escaped inner pair is fully closed; the
    /// inner group must still be reported correctly even though the outer one is `Open`.
    #[test]
    fn closed_inner_escaped_pair_survives_an_unclosed_outer_backtick() {
        let source = r"echo `echo \`rm -rf /\`";
        let result = parse(ShellDialect::Bash, source);
        assert!(matches!(result.status, ShellParseStatus::Recovered { .. }));
        let decomposed = result.decompose_for_permissions(source);
        assert!(decomposed.commands.contains(&"rm -rf /".to_string()));
    }

    /// A 2+ backslash run before a backtick represents a third nesting level and deeper, which is
    /// deliberately not modeled (see the doc comment on `detect_escaped_backtick_group`). It must
    /// not be misinterpreted as a fresh, independent single-backslash open/close elsewhere in the
    /// same text -- i.e. no nested group may be fabricated from it.
    #[test]
    fn unmodeled_deeper_backslash_run_does_not_fabricate_a_nested_group() {
        let source = r"echo `echo \\\`x\`";
        let result = parse(ShellDialect::Bash, source);
        let outer = &result.commands[0].nested_groups[0];
        assert_eq!(outer.commands.len(), 1);
        assert!(
            outer.commands[0].nested_groups.is_empty(),
            "a 3-backslash run before a backtick must not be treated as a nested escaped-backtick group"
        );
    }
}

mod shell_adapter_zsh_bash_compatibility {
    use super::*;

    #[test]
    fn posix_compatible_zsh_syntax_parses_normally() {
        let source = "echo \"$(a $(b))\" | grep x && echo done";
        let result = parse(ShellDialect::Zsh, source);
        assert_eq!(result.status, ShellParseStatus::Complete);
    }

    #[test]
    fn loud_zsh_divergences_are_rejected_as_unsupported() {
        for source in [
            "diff <(sort a) =(sort b)",
            "() { echo hi }",
            "function () { echo hi }",
            "for i (1 2 3) print $i",
            "ls *.txt(.)",
            "echo ${(f)\"$(cmd)\"}",
            "{\n  risky\n} always {\n  cleanup\n}",
            "echo $+name",
        ] {
            let result = parse(ShellDialect::Zsh, source);
            assert_eq!(
                result.status,
                ShellParseStatus::Rejected(ShellParseRejection::UnsupportedDialectSyntax),
                "expected {source:?} to be rejected, got {:?}",
                result.status
            );
            assert!(
                result.commands.is_empty(),
                "a rejected parse must return no partial hierarchy"
            );
        }
    }

    /// The one Zsh divergence the Bash grammar does *not* flag with an error: `repeat` must be
    /// caught by the command-position guard instead, and rejected the same way as a loud error.
    #[test]
    fn repeat_loop_is_rejected_despite_a_clean_bash_parse() {
        let result = parse(ShellDialect::Zsh, "repeat 3 do\necho hi\ndone");
        assert_eq!(
            result.status,
            ShellParseStatus::Rejected(ShellParseRejection::UnsupportedDialectSyntax)
        );
        assert!(result.commands.is_empty());
    }

    /// The guard must not reject `repeat` used as an ordinary argument.
    #[test]
    fn repeat_as_an_argument_is_not_rejected() {
        for source in ["echo repeat", "echo a repeat b", "printf repeat"] {
            let result = parse(ShellDialect::Zsh, source);
            assert_eq!(
                result.status,
                ShellParseStatus::Complete,
                "{source:?} should not be rejected"
            );
        }
    }

    #[test]
    fn named_directory_and_glob_are_not_flagged_as_divergences() {
        for source in ["cd ~mydir", "ls **/*.txt"] {
            let result = parse(ShellDialect::Zsh, source);
            assert_eq!(
                result.status,
                ShellParseStatus::Complete,
                "{source:?} should parse cleanly"
            );
        }
    }
}

// See `api_boundary_tests.rs` for `shell_adapter_no_backend_types_in_public_api`: it moved there
// alongside the alias-resolving checker it now uses (an earlier, purely substring-based version
// of this test lived here, but review found it could not catch a type-alias leak).
