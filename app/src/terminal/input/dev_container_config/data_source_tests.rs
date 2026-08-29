use std::path::PathBuf;

use super::DevContainerConfigSelectorDataSource;

/// `InlineMenuSelection::reset_to_best` highlights the last enabled row, so the config that
/// discovery found first has to come out last for it to be the default pick.
#[test]
fn zero_state_order_puts_first_discovered_config_last() {
    let source = DevContainerConfigSelectorDataSource::new(vec![
        PathBuf::from("/repo/.devcontainer/devcontainer.json"),
        PathBuf::from("/repo/.devcontainer/backend/devcontainer.json"),
        PathBuf::from("/repo/.devcontainer.json"),
    ]);

    let ordered: Vec<_> = source.zero_state_order().cloned().collect();

    assert_eq!(
        ordered,
        vec![
            PathBuf::from("/repo/.devcontainer.json"),
            PathBuf::from("/repo/.devcontainer/backend/devcontainer.json"),
            PathBuf::from("/repo/.devcontainer/devcontainer.json"),
        ]
    );
}

#[test]
fn zero_state_order_is_empty_without_configs() {
    let source = DevContainerConfigSelectorDataSource::new(vec![]);

    assert_eq!(source.zero_state_order().count(), 0);
}
