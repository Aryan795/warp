//! Verifies the module boundary in `specs/APP-5430/TECH.md`: no `pub` item in `shell::mod` (the
//! adapter's entire public surface -- `mapper`, where real `arborium`/`tree_sitter` usage lives,
//! is a private submodule) may name an Arborium or tree-sitter type, directly or through a local
//! `use` alias.
//!
//! An earlier version of this test matched banned crate names as a literal substring of each
//! `pub` line. Review found that check could not catch a type *alias*: `mod.rs` writes
//! `use arborium::tree_sitter::Language;` and then names the local identifier `Language` in
//! `ShellDialect::grammar`'s return type, and neither `"arborium"` nor `"tree_sitter"` appears
//! anywhere near that return type as written. [`extract_pub_items`] resolves each `use` statement
//! to a local-name -> full-path table first, then checks every identifier appearing in a `pub`
//! item's signature (return type, parameters, struct/enum fields) against both the literal
//! substrings *and* that alias table, so a locally-renamed leak is caught the same way a spelled-
//! out one is.
//!
//! This is still a text-based check, not a real type checker, so [`mutation_test_confirms_the_checker_catches_an_alias_leak`]
//! proves it actually works by mutation: it runs the same checker against a small fixture that
//! reproduces the exact alias-leak shape review found (a `pub` function returning a locally-
//! aliased `arborium::tree_sitter::Language`) and asserts the checker flags it.

#[derive(Debug, Clone, PartialEq, Eq)]
struct BannedTypeUse {
    item_summary: String,
    identifier: String,
}

/// Checks `source` (a `mod.rs`-shaped Rust source file) for any externally-visible `pub` item
/// whose signature names an Arborium or tree-sitter type, directly or via a local `use` alias
/// (`pub(crate)`/`pub(super)`/etc are not externally visible and are not checked). Returns every
/// violation found, empty if none.
fn find_banned_type_uses_in_public_api(source: &str) -> Vec<BannedTypeUse> {
    let aliases = collect_banned_type_aliases(source);
    let mut violations = Vec::new();
    for item in extract_pub_item_signatures(source) {
        for identifier in identifiers_in(&item.signature_text) {
            let is_banned_directly =
                identifier.starts_with("arborium") || identifier.starts_with("tree_sitter");
            let is_banned_via_alias = aliases.contains(&identifier);
            if is_banned_directly || is_banned_via_alias {
                violations.push(BannedTypeUse {
                    item_summary: item.summary.clone(),
                    identifier: identifier.clone(),
                });
            }
        }
    }
    violations
}

/// Builds the set of local identifiers that a `use` statement in `source` binds to an
/// `arborium`/`tree_sitter` path, e.g. `use arborium::tree_sitter::Language;` binds `Language`.
/// Does not handle wildcard imports (`use arborium::tree_sitter::*;`) or `use ... as`-renamed
/// imports of a banned path with a non-obvious local name beyond the simple `as NewName` form --
/// see the panic below, which fails loudly rather than silently under-reporting if either
/// appears, since this checker's soundness depends on every banned local name being enumerable.
fn collect_banned_type_aliases(source: &str) -> std::collections::HashSet<String> {
    let mut aliases = std::collections::HashSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("use ") {
            continue;
        }
        let is_banned_path = trimmed.contains("arborium") || trimmed.contains("tree_sitter");
        if !is_banned_path {
            continue;
        }
        assert!(
            !trimmed.contains('*'),
            "a wildcard `use` of an arborium/tree_sitter path makes this checker unsound \
             (it cannot enumerate the names a `*` import binds): {trimmed:?}"
        );
        let local_name = if let Some((_, renamed)) = trimmed.rsplit_once(" as ") {
            renamed.trim_end_matches(';').trim()
        } else {
            let path = trimmed
                .trim_start_matches("use ")
                .trim_end_matches(';')
                .trim_end_matches(|c: char| c.is_whitespace());
            path.rsplit("::").next().unwrap_or(path)
        };
        aliases.insert(local_name.to_string());
    }
    aliases
}

struct PubItemSignature {
    /// A short human-readable label for error messages (e.g. the item's first line).
    summary: String,
    /// The item's signature text: for a function, everything up to (not including) its body; for
    /// a struct/enum, its entire definition including fields, since fields are exactly what must
    /// be checked.
    signature_text: String,
}

/// Extracts every top-level, externally-visible `pub` item's signature from `source` (not
/// `pub(crate)`/`pub(super)`/etc, which are not part of the external API this test guards), using
/// brace/paren balance to find each item's extent rather than a single-line match, so a signature
/// split across multiple lines (as `ShellDialect::grammar`'s easily could be) is still captured
/// whole.
fn extract_pub_item_signatures(source: &str) -> Vec<PubItemSignature> {
    let mut items = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        // `pub(crate)`/`pub(super)`/etc. are intentionally *not* flagged: they are not reachable
        // from outside the crate, so naming a backend type there (as `ShellDialect::grammar`'s
        // real `pub(crate) fn grammar(self) -> Language` legitimately does, for `mapper.rs`'s own
        // internal use) is not the API leak this test guards against. Only a bare `pub ` item is
        // externally visible.
        let is_top_level_pub_item = !lines[i].starts_with(' ')
            && !lines[i].starts_with('\t')
            && trimmed.starts_with("pub ");
        if !is_top_level_pub_item {
            i += 1;
            continue;
        }
        let summary = trimmed.to_string();
        let mut signature_lines = Vec::new();
        let mut depth: i32 = 0;
        let mut started_body_brace = false;
        loop {
            let line = lines[i];
            // Stop consuming this item's *signature* at the `{` that opens a function body: a
            // struct/enum needs its whole `{ ... }` block (fields matter), but a function body's
            // *contents* are irrelevant to its public signature and must not be scanned (a
            // private local variable named `arborium_result` inside a function body is not a
            // public API leak).
            let is_fn_item = summary.contains("fn ");
            for ch in line.chars() {
                match ch {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '{' if is_fn_item && depth == 0 && !started_body_brace => {
                        started_body_brace = true;
                    }
                    '{' if !started_body_brace => depth += 1,
                    '}' if !started_body_brace => depth -= 1,
                    _ => {}
                }
            }
            if started_body_brace {
                // Include everything up to (not including) the opening `{` on this line.
                let before_brace = line.split('{').next().unwrap_or(line);
                signature_lines.push(before_brace.to_string());
                break;
            }
            signature_lines.push(line.to_string());
            let ends_item =
                depth <= 0 && (line.trim_end().ends_with(';') || line.trim_end().ends_with('}'));
            if ends_item {
                break;
            }
            i += 1;
            if i >= lines.len() {
                break;
            }
        }
        items.push(PubItemSignature {
            summary,
            signature_text: signature_lines.join("\n"),
        });
        i += 1;
    }
    items
}

/// Extracts every identifier-like token from `text` (ASCII word characters, i.e. what Rust
/// allows in a type/path segment name).
fn identifiers_in(text: &str) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            identifiers.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        identifiers.push(current);
    }
    identifiers
}

#[test]
fn shell_adapter_no_backend_types_in_public_api() {
    let mod_rs = include_str!("mod.rs");
    assert!(
        !mod_rs.contains("pub mod mapper"),
        "the mapper submodule (where arborium/tree_sitter usage lives) must stay private"
    );
    let violations = find_banned_type_uses_in_public_api(mod_rs);
    assert!(
        violations.is_empty(),
        "found Arborium/tree-sitter type(s) reachable from a pub item: {violations:?}"
    );
}

/// Mutation test: proves the checker in this file actually catches the exact leak shape review
/// found (a locally-aliased Arborium/tree-sitter type named in a `pub` function's return type),
/// rather than merely asserting it against the current, already-clean `mod.rs`.
#[test]
fn mutation_test_confirms_the_checker_catches_an_alias_leak() {
    let fixture = r#"
use arborium::tree_sitter::Language;

pub fn grammar(self) -> Language {
    todo!()
}
"#;
    let violations = find_banned_type_uses_in_public_api(fixture);
    assert!(
        !violations.is_empty(),
        "the checker failed to catch a public function returning a locally-aliased \
         arborium::tree_sitter::Language -- this is exactly the leak shape review found, so a \
         checker that misses it here would also miss it in the real mod.rs"
    );
    assert!(violations.iter().any(|v| v.identifier == "Language"));

    // Sanity check the other direction too: a `pub(crate)` item is *not* reachable from outside
    // the crate, so naming a backend type there -- exactly what the real, legitimate
    // `ShellDialect::grammar` does today -- must not be flagged. A checker that flagged
    // `pub(crate)` too would fail this crate's own real `mod.rs` for no reason.
    let pub_crate_fixture = r#"
use arborium::tree_sitter::Language;

pub(crate) fn grammar(self) -> Language {
    todo!()
}
"#;
    let violations = find_banned_type_uses_in_public_api(pub_crate_fixture);
    assert!(
        violations.is_empty(),
        "pub(crate) items must not be flagged -- they are not part of the external API: {violations:?}"
    );
}
