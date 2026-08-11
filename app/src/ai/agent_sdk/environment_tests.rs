use super::*;

fn secret_ref(name: &str) -> EnvironmentSecretRef {
    EnvironmentSecretRef {
        name: name.to_string(),
    }
}

fn names(secrets: &Option<Vec<EnvironmentSecretRef>>) -> Vec<&str> {
    secrets
        .as_deref()
        .expect("secrets should be set")
        .iter()
        .map(|secret| secret.name.as_str())
        .collect()
}

#[test]
fn secrets_for_create_leaves_field_absent_without_flags() {
    assert!(secrets_for_create(vec![]).is_none());
}

#[test]
fn secrets_for_create_deduplicates_and_preserves_order() {
    let secrets = secrets_for_create(vec![
        "GITHUB_TOKEN".to_string(),
        "NPM_TOKEN".to_string(),
        "GITHUB_TOKEN".to_string(),
    ]);

    assert_eq!(names(&secrets), ["GITHUB_TOKEN", "NPM_TOKEN"]);
}

#[test]
fn apply_secret_flags_without_flags_leaves_secrets_unchanged() {
    let update = apply_secret_flags(Some(&[secret_ref("GITHUB_TOKEN")]), SecretFlags::default())
        .expect("no flags is valid");

    assert!(update.is_none());
}

#[test]
fn apply_secret_flags_adds_and_removes() {
    let update = apply_secret_flags(
        Some(&[secret_ref("OLD_TOKEN"), secret_ref("KEEP_TOKEN")]),
        SecretFlags {
            add: vec!["NEW_TOKEN".to_string()],
            remove: vec!["OLD_TOKEN".to_string()],
            remove_all: false,
        },
    )
    .expect("deltas are valid")
    .expect("deltas produce an update");

    assert_eq!(names(&update.secrets), ["KEEP_TOKEN", "NEW_TOKEN"]);
    assert!(update.missing_removals.is_empty());
    assert!(!update.narrowed_from_all_secrets);
}

#[test]
fn apply_secret_flags_reports_removals_that_were_not_configured() {
    let update = apply_secret_flags(
        Some(&[secret_ref("KEEP_TOKEN")]),
        SecretFlags {
            add: vec![],
            remove: vec!["MISSING_TOKEN".to_string()],
            remove_all: false,
        },
    )
    .expect("deltas are valid")
    .expect("deltas produce an update");

    assert_eq!(names(&update.secrets), ["KEEP_TOKEN"]);
    assert_eq!(update.missing_removals, ["MISSING_TOKEN"]);
}

#[test]
fn apply_secret_flags_does_not_duplicate_an_already_configured_secret() {
    let update = apply_secret_flags(
        Some(&[secret_ref("GITHUB_TOKEN")]),
        SecretFlags {
            add: vec!["GITHUB_TOKEN".to_string()],
            remove: vec![],
            remove_all: false,
        },
    )
    .expect("deltas are valid")
    .expect("deltas produce an update");

    assert_eq!(names(&update.secrets), ["GITHUB_TOKEN"]);
}

#[test]
fn apply_secret_flags_narrows_an_environment_that_exposed_every_secret() {
    let update = apply_secret_flags(
        None,
        SecretFlags {
            add: vec!["GITHUB_TOKEN".to_string()],
            remove: vec![],
            remove_all: false,
        },
    )
    .expect("adding to an unrestricted environment is valid")
    .expect("deltas produce an update");

    assert_eq!(names(&update.secrets), ["GITHUB_TOKEN"]);
    assert!(update.narrowed_from_all_secrets);
}

#[test]
fn apply_secret_flags_rejects_removal_from_an_environment_that_exposed_every_secret() {
    let err = apply_secret_flags(
        None,
        SecretFlags {
            add: vec![],
            remove: vec!["GITHUB_TOKEN".to_string()],
            remove_all: false,
        },
    )
    .expect_err("there is no secret list to remove from");

    assert!(err.to_string().contains("--remove-all-secrets"), "{err}");
}

#[test]
fn apply_secret_flags_remove_all_produces_an_empty_list() {
    let update = apply_secret_flags(
        Some(&[secret_ref("GITHUB_TOKEN")]),
        SecretFlags {
            add: vec![],
            remove: vec![],
            remove_all: true,
        },
    )
    .expect("remove-all is valid")
    .expect("remove-all produces an update");

    // `Some([])` exposes no secrets; `None` would restore "every available
    // secret", which is the opposite of what the flag asks for.
    assert_eq!(update.secrets, Some(vec![]));
}

#[test]
fn apply_secret_flags_remove_all_is_valid_for_an_environment_that_exposed_every_secret() {
    let update = apply_secret_flags(
        None,
        SecretFlags {
            add: vec![],
            remove: vec![],
            remove_all: true,
        },
    )
    .expect("remove-all is valid")
    .expect("remove-all produces an update");

    assert_eq!(update.secrets, Some(vec![]));
    assert!(!update.narrowed_from_all_secrets);
}

#[test]
fn display_secrets_distinguishes_unrestricted_from_empty() {
    assert_eq!(display_secrets(None), "All available secrets");
    assert_eq!(display_secrets(Some(&[])), "None");
    assert_eq!(
        display_secrets(Some(&[secret_ref("A"), secret_ref("B")])),
        "A, B"
    );
}

#[test]
fn environment_list_table_row_matches_header() {
    let header = EnvironmentInfo::header()
        .into_iter()
        .map(|cell| cell.content().to_string())
        .collect::<Vec<_>>();
    let info = EnvironmentInfo {
        id: "env-1".to_string(),
        name: "env".to_string(),
        description: None,
        base_image: None,
        github_repos: vec![],
        setup_commands: vec![],
        secrets: Some(vec![secret_ref("GITHUB_TOKEN")]),
        creator_email: "user@warp.dev".to_string(),
        last_edited: "1 day ago".to_string(),
        last_edited_utc: None,
        scope: "Personal".to_string(),
    };
    let row = info
        .row()
        .into_iter()
        .map(|cell| cell.content().to_string())
        .collect::<Vec<_>>();

    assert_eq!(row.len(), header.len());
    let secrets_index = header
        .iter()
        .position(|column| column == "Secrets")
        .expect("list output has a Secrets column");
    assert_eq!(row[secrets_index], "GITHUB_TOKEN");
}
