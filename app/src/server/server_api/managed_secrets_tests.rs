use warp_graphql::managed_secrets::{ManagedSecret, ManagedSecretType};
use warp_graphql::object::{Space, SpaceType};

use super::retain_personal_and_team_secrets;

fn secret(name: &str, owner_type: SpaceType, owner_uid: &str) -> ManagedSecret {
    ManagedSecret {
        name: name.to_string(),
        description: None,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
        owner: Space {
            uid: cynic::Id::new(owner_uid),
            type_: owner_type,
        },
        type_: ManagedSecretType::RawValue,
    }
}

#[test]
fn retains_only_personal_secrets_when_no_team_selected() {
    let secrets = vec![
        secret("personal", SpaceType::User, "user-uid"),
        secret("team-a", SpaceType::Team, "team-a-uid"),
    ];

    let retained = retain_personal_and_team_secrets(secrets, None);

    assert_eq!(
        retained.into_iter().map(|s| s.name).collect::<Vec<_>>(),
        vec!["personal".to_string()]
    );
}

#[test]
fn retains_personal_and_selected_team_secrets_but_not_other_teams() {
    let secrets = vec![
        secret("personal", SpaceType::User, "user-uid"),
        secret("team-a", SpaceType::Team, "team-a-uid"),
        secret("team-b", SpaceType::Team, "team-b-uid"),
    ];

    let retained = retain_personal_and_team_secrets(secrets, Some("team-a-uid"));

    assert_eq!(
        retained.into_iter().map(|s| s.name).collect::<Vec<_>>(),
        vec!["personal".to_string(), "team-a".to_string()]
    );
}
