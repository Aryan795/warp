use chrono::Utc;
use warp_graphql::mutations::update_user_settings::UpdateUserSettingsResult;
use warp_graphql::object_permissions::OwnerType;
use warp_graphql::queries::api_keys::ApiKeyProperties;

use super::{AuthClientImpl, retain_personal_and_team_api_keys};

#[test]
fn unknown_settings_results_preserve_operation_context() {
    for expected_message in [
        "failed to set telemetry enabled",
        "failed to set crash reporting enabled",
        "failed to set cloud conversation storage enabled",
        "failed to update user settings",
    ] {
        let error = AuthClientImpl::on_settings_updated(
            UpdateUserSettingsResult::Unknown,
            expected_message,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), expected_message);
    }
}

fn api_key(name: &str, owner_type: OwnerType) -> ApiKeyProperties {
    let now = warp_graphql::scalars::Time::from(Utc::now());
    ApiKeyProperties {
        uid: cynic::Id::new(name),
        name: name.to_string(),
        key_suffix: "abcd".to_string(),
        owner_type,
        agent_info: None,
        expires_at: None,
        last_used_at: None,
        created_at: now,
    }
}

#[test]
fn retains_only_personal_keys_when_no_team_selected() {
    let keys = vec![
        api_key("personal", OwnerType::User),
        api_key("team", OwnerType::Team),
    ];

    let retained = retain_personal_and_team_api_keys(keys, None);

    assert_eq!(
        retained.into_iter().map(|k| k.name).collect::<Vec<_>>(),
        vec!["personal".to_string()]
    );
}

#[test]
fn retains_personal_and_team_keys_when_team_selected() {
    let keys = vec![
        api_key("personal", OwnerType::User),
        api_key("team", OwnerType::Team),
    ];

    let retained = retain_personal_and_team_api_keys(keys, Some("some-team-uid"));

    let mut names: Vec<_> = retained.into_iter().map(|k| k.name).collect();
    names.sort();
    assert_eq!(names, vec!["personal".to_string(), "team".to_string()]);
}
