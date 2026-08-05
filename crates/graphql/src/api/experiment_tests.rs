/// The schema snapshot the cynic enum is generated against. Reading it here
/// makes the cross-repo contract with warp-server an explicit assertion rather
/// than an implicit build-time detail.
const SCHEMA: &str = include_str!("../../../warp_graphql_schema/api/schema.graphql");

/// REV-1939: the client can only read the arm the server assigns if both sides
/// spell it identically, so pin the two server-owned values exactly.
#[test]
fn schema_exposes_exactly_the_two_choose_how_to_start_arms() {
    let arms: Vec<&str> = SCHEMA
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("ONBOARDING_CHOOSE_HOW_TO_START"))
        .collect();

    assert_eq!(
        arms,
        vec![
            "ONBOARDING_CHOOSE_HOW_TO_START_CONTROL",
            "ONBOARDING_CHOOSE_HOW_TO_START_THREE_OPTIONS",
        ]
    );
}
