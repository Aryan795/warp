//! Guards the checked-in SDL against the codegen allowlists in
//! `api/client-schema.ts`.
//!
//! `client-schema.ts` filters the introspected server schema down to the root
//! fields listed in its allowlists. A root field that is present in
//! `api/schema.graphql` but missing from the matching allowlist is silently
//! dropped (along with its now-unreferenced types) the next time anyone runs
//! `yarn generate`, breaking every client operation that used it.
//!
//! Both parsers below work line by line via [`str::lines`], which accepts `\n`
//! and `\r\n` alike. The sources are embedded with `include_str!`, so on a
//! Windows checkout they carry CRLF endings and any parser that searched for
//! `\n`-delimited substrings would find nothing.

const SCHEMA_SDL: &str = include_str!("../api/schema.graphql");
const CLIENT_SCHEMA_TS: &str = include_str!("../api/client-schema.ts");

/// Collects the field names of the named root type from an SDL document,
/// skipping `"""`-delimited descriptions and nested argument lines.
fn root_fields<'a>(sdl: &'a str, type_name: &str) -> Vec<&'a str> {
    let header = format!("type {type_name} {{");
    let mut lines = sdl.lines();
    assert!(
        lines.any(|line| line == header),
        "`{header}` not found in the schema"
    );

    let mut fields = Vec::new();
    let mut in_description = false;
    let mut terminated = false;
    for line in lines {
        if line == "}" {
            terminated = true;
            break;
        }
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
    assert!(terminated, "`{header}` block is not terminated");
    fields
}

/// Collects the string literals of the named `const <name> = [...]` array from
/// a TypeScript source. The array may be declared on one line or span several.
fn allowlist<'a>(source: &'a str, const_name: &str) -> Vec<&'a str> {
    let header = format!("const {const_name} = [");
    let mut lines = source.lines();
    let mut segment = lines
        .find_map(|line| line.strip_prefix(header.as_str()))
        .unwrap_or_else(|| panic!("`{header}` not found in the client schema loader"));

    let mut entries = Vec::new();
    loop {
        let (body, closed) = match segment.split_once(']') {
            Some((before_bracket, _)) => (before_bracket, true),
            None => (segment, false),
        };
        entries.extend(body.split('\'').skip(1).step_by(2));
        if closed {
            return entries;
        }
        segment = lines
            .next()
            .unwrap_or_else(|| panic!("`{header}` array is not terminated"));
    }
}

fn assert_root_fields_allowlisted(type_name: &str, const_name: &str) {
    let allowed = allowlist(CLIENT_SCHEMA_TS, const_name);
    let missing: Vec<&str> = root_fields(SCHEMA_SDL, type_name)
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

const SAMPLE_SDL: &str = r#"type Other {
  ignored: Boolean!
}

type RootQuery {
  """
  Note: descriptions can contain colons and look like fields.
  """
  user(uid: ID!): UserResult!
  """A single-line description."""
  apiKeys: ApiKeysResult!
  paged(
    first: Int
  ): PagedResult!
}
"#;

const SAMPLE_TS: &str = r#"const clientQueries = [
  'user',
  'apiKeys',
  'paged',
];

const clientSubscriptions = ['updates'];
"#;

fn as_crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

#[test]
fn root_fields_parse_with_either_line_ending() {
    let expected = vec!["user", "apiKeys", "paged"];
    assert_eq!(root_fields(SAMPLE_SDL, "RootQuery"), expected);

    let crlf_sdl = as_crlf(SAMPLE_SDL);
    assert_eq!(root_fields(&crlf_sdl, "RootQuery"), expected);
}

#[test]
fn allowlist_parses_with_either_line_ending() {
    let expected = vec!["user", "apiKeys", "paged"];
    assert_eq!(allowlist(SAMPLE_TS, "clientQueries"), expected);

    let crlf_ts = as_crlf(SAMPLE_TS);
    assert_eq!(allowlist(&crlf_ts, "clientQueries"), expected);
}

#[test]
fn allowlist_parses_single_line_declarations() {
    let expected = vec!["updates"];
    assert_eq!(allowlist(SAMPLE_TS, "clientSubscriptions"), expected);

    let crlf_ts = as_crlf(SAMPLE_TS);
    assert_eq!(allowlist(&crlf_ts, "clientSubscriptions"), expected);
}
