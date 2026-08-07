//! The Rust half of the shared Agent Plugins conformance corpus.
//!
//! The corpus is canonical in `warpdotdev/warp-server` and vendored verbatim beside this file;
//! see `contract/README.md`. warp-server's Go validator runs the same fixtures with the same
//! expectations, so the two implementations cannot silently diverge on what they accept — a
//! package that Factory sync admits and the runtime then disables is precisely the failure this
//! guards against.
//!
//! **Outcomes are compared, never diagnostic codes or wording.** The two implementations have
//! different diagnostic vocabularies by design, and pinning messages across repositories would
//! make the corpus fail on cosmetic changes while still missing real behavioral drift.
use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use super::diagnostics::PluginDiagnosticSeverity;
use super::manifest::{AGENT_PLUGINS_VERSION_1_0_0, parse_manifest};
use super::mcp::parse_plugin_mcp;

const CORPUS_DIR: &str = "src/plugins/contract/agentplugins-conformance";
const CASES: &str = include_str!("contract/agentplugins-conformance/cases.json");

/// One fixture and the outcome both validators must produce for it.
#[derive(Debug, Deserialize)]
struct ConformanceCase {
    name: String,
    kind: String,
    path: String,
    expect: String,
    #[allow(dead_code)]
    reason: String,
}

/// The outcome vocabulary the corpus declares, in `README.md`.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Accepted with no diagnostic at all.
    Valid,
    /// Accepted with at least one non-blocking diagnostic — the standard's report-and-continue
    /// behavior. Rejecting one of these would be non-conformant.
    Warn,
    /// Not usable: either the document was rejected outright, or the single server it declares
    /// was disabled.
    Invalid,
}

impl Outcome {
    fn parse(expect: &str) -> Self {
        match expect {
            "valid" => Outcome::Valid,
            "warn" => Outcome::Warn,
            "invalid" => Outcome::Invalid,
            other => panic!("unknown expectation '{other}' in the conformance corpus"),
        }
    }
}

/// Classifies diagnostics by severity rather than by code.
///
/// Severity is the outcome-level property the corpus's `warn` versus `invalid` distinction
/// actually rests on: an unsupported transport is skipped and reported without blocking, while an
/// invalid entry disables the server. Both leave zero usable servers behind, so severity is what
/// separates them without reaching for a message.
fn classify(diagnostics: &[super::diagnostics::PluginDiagnostic]) -> Outcome {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == PluginDiagnosticSeverity::Error)
    {
        Outcome::Invalid
    } else if diagnostics.is_empty() {
        Outcome::Valid
    } else {
        Outcome::Warn
    }
}

fn corpus_path(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CORPUS_DIR)
        .join(relative)
}

fn cases() -> Vec<ConformanceCase> {
    serde_json::from_str(CASES).expect("the vendored cases.json parses")
}

/// Runs every fixture through the validator its `kind` selects.
#[test]
fn the_shared_conformance_corpus_passes() {
    let mut failures: Vec<String> = Vec::new();

    for case in cases() {
        let content = std::fs::read_to_string(corpus_path(&case.path))
            .unwrap_or_else(|error| panic!("fixture '{}' is missing: {error}", case.path));
        let expected = Outcome::parse(&case.expect);

        let actual = match case.kind.as_str() {
            "manifest" => match parse_manifest(&content) {
                Err(_) => Outcome::Invalid,
                Ok(parsed) => classify(&parsed.diagnostics),
            },
            "mcp" => match parse_plugin_mcp(&content, AGENT_PLUGINS_VERSION_1_0_0) {
                Err(_) => Outcome::Invalid,
                Ok(parsed) => classify(&parsed.diagnostics),
            },
            other => panic!("unknown case kind '{other}' in the conformance corpus"),
        };

        if actual != expected {
            failures.push(format!(
                "  {} ({}): expected {:?}, got {:?} — {}",
                case.name, case.kind, expected, actual, case.reason
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the Rust validator diverged from the shared corpus on {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every fixture on disk is declared, and every declaration points at a file that exists.
///
/// warp-server asserts the same thing on its copy. Without it a fixture could be added on one
/// side with no expectation, which is how a corpus quietly stops covering what it claims to.
#[test]
fn every_fixture_is_declared_exactly_once() {
    let declared: Vec<String> = cases().into_iter().map(|case| case.path).collect();
    let unique: BTreeSet<&String> = declared.iter().collect();
    assert_eq!(
        declared.len(),
        unique.len(),
        "a fixture is declared more than once in cases.json"
    );

    let mut on_disk: BTreeSet<String> = BTreeSet::new();
    for kind in ["manifest", "mcp"] {
        let dir = corpus_path(kind);
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!("corpus directory {} is missing: {error}", dir.display())
        }) {
            let entry = entry.expect("a readable corpus directory entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") {
                on_disk.insert(format!("{kind}/{name}"));
            }
        }
    }

    let declared: BTreeSet<String> = declared.into_iter().collect();
    let undeclared: Vec<&String> = on_disk.difference(&declared).collect();
    let missing: Vec<&String> = declared.difference(&on_disk).collect();

    assert!(
        undeclared.is_empty(),
        "fixtures exist with no expectation in cases.json: {undeclared:?}"
    );
    assert!(
        missing.is_empty(),
        "cases.json declares fixtures that do not exist: {missing:?}"
    );
}
