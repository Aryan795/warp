use warp_graphql::object::Space;
use warp_graphql::scalars::Time;

use super::{
    Runner, RunnerArch, RunnerArchArg, RunnerConfig, RunnerOs, RunnerOsArg, SpaceType,
    confirm_delete, merge_instance_shape, resolve_arch, resolve_runner, resolve_updated_name,
};

fn make_runner(uid: &str, name: &str) -> Runner {
    Runner {
        uid: cynic::Id::new(uid),
        config: RunnerConfig {
            name: name.to_string(),
            description: None,
            setup_commands: None,
            instance_shape: None,
            os: RunnerOs::Linux,
            arch: RunnerArch::X8664,
            mac: None,
            linux: None,
        },
        last_updated: Time::new(chrono::Utc::now()),
        scope: Space {
            uid: cynic::Id::new("space-uid"),
            type_: SpaceType::User,
        },
        creator: None,
        last_editor: None,
    }
}

#[test]
fn confirm_delete_refuses_non_interactive_without_force() {
    // In non-interactive mode, refusal must surface as an error so the caller
    // exits non-zero instead of treating a skipped delete as a success.
    let err = confirm_delete("runner-123", false).expect_err("non-interactive refusal is an error");
    let msg = err.to_string();
    assert!(msg.contains("non-interactive"), "got: {msg}");
    assert!(msg.contains("runner-123"), "got: {msg}");
}

#[test]
fn resolve_arch_auto_maps_to_os_default() {
    assert!(matches!(
        resolve_arch(RunnerArchArg::Auto, RunnerOsArg::Linux),
        RunnerArch::X8664
    ));
    assert!(matches!(
        resolve_arch(RunnerArchArg::Auto, RunnerOsArg::Macos),
        RunnerArch::Aarch64
    ));
}

#[test]
fn resolve_arch_explicit_is_preserved_regardless_of_os() {
    assert!(matches!(
        resolve_arch(RunnerArchArg::X8664, RunnerOsArg::Macos),
        RunnerArch::X8664
    ));
    assert!(matches!(
        resolve_arch(RunnerArchArg::Aarch64, RunnerOsArg::Linux),
        RunnerArch::Aarch64
    ));
}

#[test]
fn merge_instance_shape_updates_dimensions_independently() {
    // Neither specified: preserve the existing shape.
    assert_eq!(
        merge_instance_shape(None, None, Some((2, 4))).unwrap(),
        Some((2, 4))
    );
    // Only vCPUs: keep existing memory.
    assert_eq!(
        merge_instance_shape(Some(8), None, Some((2, 4))).unwrap(),
        Some((8, 4))
    );
    // Only memory: keep existing vCPUs.
    assert_eq!(
        merge_instance_shape(None, Some(16), Some((2, 4))).unwrap(),
        Some((2, 16))
    );
    // Both specified: use both.
    assert_eq!(
        merge_instance_shape(Some(8), Some(16), Some((2, 4))).unwrap(),
        Some((8, 16))
    );
    // No existing shape and nothing set: no shape.
    assert_eq!(merge_instance_shape(None, None, None).unwrap(), None);
}

#[test]
fn merge_instance_shape_errors_on_partial_shape_without_existing() {
    assert!(merge_instance_shape(Some(8), None, None).is_err());
    assert!(merge_instance_shape(None, Some(16), None).is_err());
}

#[test]
fn resolve_updated_name_renames_only_with_uid() {
    // UID + --name renames the runner.
    assert_eq!(resolve_updated_name(true, Some("new"), "old"), "new");
    // UID without --name keeps the existing name.
    assert_eq!(resolve_updated_name(true, None, "old"), "old");
    // No UID: --name is the selector, so the name is unchanged.
    assert_eq!(resolve_updated_name(false, Some("old"), "old"), "old");
}

#[test]
fn resolve_runner_matches_exact_uid() {
    let runners = vec![make_runner("uid-1", "alpha"), make_runner("uid-2", "beta")];
    let resolved = resolve_runner(&runners, Some("uid-2"), None).unwrap();
    assert_eq!(resolved.uid.inner(), "uid-2");
}

#[test]
fn resolve_runner_falls_back_to_name_when_identifier_is_not_a_uid() {
    let runners = vec![make_runner("uid-1", "alpha"), make_runner("uid-2", "beta")];
    let resolved = resolve_runner(&runners, Some("beta"), None).unwrap();
    assert_eq!(resolved.uid.inner(), "uid-2");
}

#[test]
fn resolve_runner_prefers_uid_match_over_name_param() {
    let runners = vec![make_runner("uid-1", "alpha"), make_runner("uid-2", "beta")];
    let resolved = resolve_runner(&runners, Some("uid-1"), Some("new-name")).unwrap();
    assert_eq!(resolved.uid.inner(), "uid-1");
}

#[test]
fn resolve_runner_uses_name_arg_when_identifier_is_absent() {
    let runners = vec![make_runner("uid-1", "alpha")];
    let resolved = resolve_runner(&runners, None, Some("alpha")).unwrap();
    assert_eq!(resolved.uid.inner(), "uid-1");
}

#[test]
fn resolve_runner_errors_when_name_is_ambiguous() {
    let runners = vec![make_runner("uid-1", "dup"), make_runner("uid-2", "dup")];
    let err = resolve_runner(&runners, Some("dup"), None).unwrap_err();
    assert!(
        err.to_string().contains("Multiple runners match"),
        "got: {err}"
    );
}

#[test]
fn resolve_runner_errors_when_not_found() {
    let runners = vec![make_runner("uid-1", "alpha")];
    let err = resolve_runner(&runners, Some("missing"), None).unwrap_err();
    assert!(err.to_string().contains("not found"), "got: {err}");
}

#[test]
fn resolve_runner_errors_when_neither_identifier_nor_name_given() {
    let runners = vec![make_runner("uid-1", "alpha")];
    let err = resolve_runner(&runners, None, None).unwrap_err();
    assert!(err.to_string().contains("required"), "got: {err}");
}
