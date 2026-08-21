use anyhow::{Ok, Result};
use vec1::vec1;
use warpui::App;
use warpui::keymap::{EditableBinding, Keystroke};

use crate::keyboard::{
    CustomKeybindings, PersistedTrigger, REMOVED_KEYBINDING_SERIALIZATION, UserDefinedKeybinding,
    load_custom_keybindings_from_path,
};
use crate::util::bindings::keybinding_name_to_display_string;
use crate::workspace::WorkspaceAction;

#[test]
fn test_short_user_defined_keybinding_to_persisted_trigger() {
    let keystroke = Keystroke::parse("ctrl-p").unwrap();
    let keybinding = UserDefinedKeybinding::Keystrokes(vec1![keystroke]);
    let persisted_trigger: PersistedTrigger = keybinding.into();

    assert_eq!(persisted_trigger, PersistedTrigger("ctrl-p".to_string()));
}

#[test]
fn test_long_user_defined_keybinding_to_persisted_trigger() {
    let keystroke = Keystroke::parse("ctrl-p").unwrap();
    let other_keystroke = Keystroke::parse("1").unwrap();

    let keybinding = UserDefinedKeybinding::Keystrokes(vec1![keystroke, other_keystroke]);
    let persisted_trigger: PersistedTrigger = keybinding.into();

    assert_eq!(persisted_trigger, PersistedTrigger("ctrl-p 1".to_string()));
}

#[test]
fn test_short_persisted_trigger_to_user_defined_keybinding() -> Result<()> {
    let persisted_trigger = PersistedTrigger("ctrl-x".to_string());
    let keybinding = UserDefinedKeybinding::try_from(persisted_trigger)?;

    let correct_keybinding =
        UserDefinedKeybinding::Keystrokes(vec1![Keystroke::parse("ctrl-x").unwrap()]);

    assert_eq!(keybinding, correct_keybinding);
    Ok(())
}

#[test]
fn test_long_persisted_trigger_to_user_defined_keybinding() -> Result<()> {
    let persisted_trigger = PersistedTrigger("ctrl-x 8".to_string());
    let keybinding = UserDefinedKeybinding::try_from(persisted_trigger)?;

    let correct_keybinding = UserDefinedKeybinding::Keystrokes(vec1![
        Keystroke::parse("ctrl-x").unwrap(),
        Keystroke::parse("8").unwrap()
    ]);

    assert_eq!(keybinding, correct_keybinding);
    Ok(())
}

#[test]
fn test_persisted_trigger_to_removed_user_keybinding() -> Result<()> {
    let persisted_trigger = PersistedTrigger(REMOVED_KEYBINDING_SERIALIZATION.to_string());
    let keybinding = UserDefinedKeybinding::try_from(persisted_trigger)?;

    assert_eq!(keybinding, UserDefinedKeybinding::Removed);
    Ok(())
}

#[test]
fn test_removed_user_keybinding_to_persisted_trigger() {
    let keybinding = UserDefinedKeybinding::Removed;
    let persisted_trigger: PersistedTrigger = keybinding.into();

    assert_eq!(
        persisted_trigger,
        PersistedTrigger(REMOVED_KEYBINDING_SERIALIZATION.to_string())
    );
}

#[test]
fn test_unparsable_persisted_trigger() {
    let persisted_trigger = PersistedTrigger("".to_string());
    let keybinding = UserDefinedKeybinding::try_from(persisted_trigger);

    assert!(keybinding.is_err());
}

/// Covers the issue's persistence acceptance criteria using the same map/serialization types
/// `write_custom_keybinding`/`remove_custom_keybinding` operate on. The disk-write step itself
/// (`crate::util::file::create_file`) always errors under `#[cfg(test)]` by design elsewhere in
/// this crate, so it can't be exercised from a unit test; this test instead drives the map
/// mutation, the real (de)serialization format, and the real load path directly.
#[test]
fn test_removing_keybinding_entry_omits_it_from_yaml_and_reload_keeps_default() {
    const BINDING_NAME: &str = "workspace:show_settings";
    const DEFAULT_KEYSTROKE: &str = "cmd-,";

    // After Clear, `write_custom_keybinding` inserts a `none` tombstone into the map.
    let mut map = CustomKeybindings::default();
    map.0.insert(
        BINDING_NAME.to_string(),
        UserDefinedKeybinding::Removed.into(),
    );
    let yaml_with_tombstone = serde_yaml::to_string(&map).expect("map should serialize");
    assert!(yaml_with_tombstone.contains(REMOVED_KEYBINDING_SERIALIZATION));

    // After Default, `remove_custom_keybinding` removes the entry from the map entirely, rather
    // than blanking it out or leaving it in some other form.
    let mut map: CustomKeybindings =
        serde_yaml::from_str(&yaml_with_tombstone).expect("map should deserialize");
    map.0.remove(BINDING_NAME);
    let yaml_after_default = serde_yaml::to_string(&map).expect("map should serialize");
    assert!(
        !yaml_after_default.contains(BINDING_NAME),
        "keybindings.yaml should no longer reference {BINDING_NAME} after Default"
    );

    // Restarting Warp loads this file fresh. With no entry for the binding, load must not
    // apply `Trigger::Empty`, so the binding's real default remains effective.
    let dir = tempfile::tempdir().expect("should create temp dir");
    let path = dir.path().join("keybindings.yaml");
    std::fs::write(&path, yaml_after_default).expect("should write fixture file");

    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.register_editable_bindings([EditableBinding::new(
                BINDING_NAME,
                "Open settings",
                WorkspaceAction::ShowSettings,
            )
            .with_key_binding(DEFAULT_KEYSTROKE)]);

            load_custom_keybindings_from_path(&path, ctx);

            assert_eq!(
                keybinding_name_to_display_string(BINDING_NAME, ctx),
                Some(Keystroke::parse(DEFAULT_KEYSTROKE).unwrap().displayed()),
                "the default keystroke should still be effective after reloading from disk"
            );
        });
    });
}
