use tempfile::tempdir;

use super::*;
use crate::plugins::identity::{PluginScopeId, PluginSourceId, PluginSourceKind};

fn instance(
    scope: PluginScopeId,
    kind: PluginSourceKind,
    identity: &str,
    name: &str,
) -> PluginInstanceId {
    PluginInstanceId::new(scope, PluginSourceId::new(kind, identity), name)
}

fn user_instance(name: &str) -> PluginInstanceId {
    instance(
        PluginScopeId::User,
        PluginSourceKind::AgentsDirectory,
        "/home/alex/.agents",
        name,
    )
}

#[test]
fn the_data_directory_is_outside_the_package_and_under_the_locator_root() {
    let locator = LocalPluginDataLocator::new("/data", PluginFrontend::Gui);
    let dir = locator.data_dir(&user_instance("devtools"));
    assert!(dir.starts_with("/data/plugins/data"));
    assert_eq!(dir.parent().unwrap(), locator.root());
}

/// §9.1: the directory is dedicated to one instance and survives package changes, so the key must
/// depend on identity that does not change with the package's contents or version.
#[test]
fn the_key_is_stable_for_one_instance() {
    let first = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("devtools"));
    let second = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("devtools"));
    assert_eq!(first, second);
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn the_key_separates_instances_that_must_not_share_data() {
    let baseline = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("devtools"));

    // A different front-end must not share writable state or running processes.
    let other_frontend = plugin_data_instance_key(PluginFrontend::Tui, &user_instance("devtools"));
    // A different plugin name.
    let other_name = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("other"));
    // The same name in a repository rather than the user's home.
    let other_scope = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repos/one",
            "devtools",
        ),
    );
    // The same name in a different repository.
    let other_repository = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repos/two",
            "devtools",
        ),
    );
    // The same repository, but the `.warp` provider rather than `.agents`.
    let other_provider = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Repository,
            PluginSourceKind::WarpDirectory,
            "/repos/one",
            "devtools",
        ),
    );

    let keys = [
        baseline,
        other_frontend,
        other_name,
        other_scope,
        other_repository,
        other_provider,
    ];
    for (index, key) in keys.iter().enumerate() {
        for (other_index, other) in keys.iter().enumerate() {
            if index != other_index {
                assert_ne!(key, other, "keys {index} and {other_index} must differ");
            }
        }
    }
}

/// Two field values that concatenate to the same bytes must still produce different keys.
#[test]
fn field_boundaries_cannot_be_confused() {
    let first = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Agent {
                name: "a".to_owned(),
            },
            PluginSourceKind::FactoryRepository,
            "/factory",
            "b",
        ),
    );
    let second = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Agent {
                name: "a/b".to_owned(),
            },
            PluginSourceKind::FactoryRepository,
            "/factory",
            "",
        ),
    );
    assert_ne!(first, second);
}

#[test]
fn ensure_data_dir_creates_the_directory() {
    let temp = tempdir().unwrap();
    let locator = LocalPluginDataLocator::new(temp.path(), PluginFrontend::Gui);
    let instance = user_instance("devtools");

    assert!(!locator.data_dir(&instance).exists());
    let created = locator.ensure_data_dir(&instance).unwrap();
    assert!(created.is_dir());
    assert_eq!(created, locator.data_dir(&instance));
}
