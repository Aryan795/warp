//! Guards the checked-in SDL against the codegen allowlists in
//! `api/client-schema.ts`.
//!
//! `client-schema.ts` filters the introspected server schema down to the root
//! fields listed in its allowlists. A root field that is present in
//! `api/schema.graphql` but missing from the matching allowlist is silently
//! dropped (along with its now-unreferenced types) the next time anyone runs
//! `yarn generate`, breaking every client operation that used it.

const SCHEMA_SDL: &str = include_str!("../api/schema.graphql");
const CLIENT_SCHEMA_TS: &str = include_str!("../api/client-schema.ts");

/// Collects the field names of the named root type from the SDL, skipping
/// `"""`-delimited descriptions and nested argument lines.
fn root_fields(type_name: &str) -> Vec<&'static str> {
    let header = format!("type {type_name} {{");
    let body = SCHEMA_SDL
        .split_once(&format!("\n{header}\n"))
        .unwrap_or_else(|| panic!("{header} not found in api/schema.graphql"))
        .1
        .split_once("\n}\n")
        .expect("root type block should be terminated")
        .0;

    let mut fields = Vec::new();
    let mut in_description = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("\"\"\"") {
            // A single-line `"""text"""` opens and closes in one go.
            if !(trimmed.len() > 3 && trimmed.ends_with("\"\"\"")) {
                in_description = !in_description;
            }
            continue;
        }
        if in_description {
            continue;
        }
        // Field definitions sit at exactly one level of indentation; argument
        // lines within a multi-line field are indented further.
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if rest.starts_with(char::is_whitespace) {
            continue;
        }
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(0);
        let (name, after) = rest.split_at(name_len);
        if !name.is_empty() && (after.starts_with('(') || after.starts_with(':')) {
            fields.push(name);
        }
    }
    fields
}

/// Collects the string literals of the named `const <name> = [...]` array from
/// `client-schema.ts`.
fn allowlist(const_name: &str) -> Vec<&'static str> {
    let body = CLIENT_SCHEMA_TS
        .split_once(&format!("const {const_name} = ["))
        .unwrap_or_else(|| panic!("const {const_name} not found in api/client-schema.ts"))
        .1
        .split_once(']')
        .expect("allowlist should be terminated")
        .0;

    body.split('\'')
        .skip(1)
        .step_by(2)
        .collect::<Vec<&'static str>>()
}

fn assert_root_fields_allowlisted(type_name: &str, const_name: &str) {
    let allowed = allowlist(const_name);
    let missing: Vec<&str> = root_fields(type_name)
        .into_iter()
        .filter(|field| !allowed.contains(field))
        .collect();
    assert!(
        missing.is_empty(),
        "{type_name} fields in api/schema.graphql are missing from `{const_name}` in \
         api/client-schema.ts and would be dropped by the next `yarn generate`: {missing:?}"
    );
}

#[test]
fn every_checked_in_query_field_is_allowlisted() {
    assert_root_fields_allowlisted("RootQuery", "clientQueries");
}

#[test]
fn every_checked_in_mutation_field_is_allowlisted() {
    assert_root_fields_allowlisted("RootMutation", "clientMutations");
}

#[test]
fn every_checked_in_subscription_field_is_allowlisted() {
    assert_root_fields_allowlisted("RootSubscription", "clientSubscriptions");
}
