use onboarding::ChooseHowToStartExperimentArm;
use warp_graphql::experiment::Experiment;
use warpui::{App, Entity, SingletonEntity};

use super::{ServerExperiment, ServerExperiments};
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

/// A model for testing purposes only.
///
/// We use it to demonstrate how client-side
/// models can be mutated to reflect server
/// experiment state changes.
pub struct TestModel(pub usize);

impl Entity for TestModel {
    type Event = ();
}
impl SingletonEntity for TestModel {}

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
}

#[test]
fn test_new_from_cached() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let model = app.add_singleton_model(|_| TestModel(0));
        let cache = vec![ServerExperiment::TestExperiment];
        app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(cache, ctx));

        // The experiment should have been enabled.
        model.read(&app, |model, _| {
            assert_eq!(model.0, 1);
        });
    });
}

#[test]
fn test_apply_latest_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let model = app.add_singleton_model(|_| TestModel(0));
        let experiments =
            app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));

        // Enable the experiment.
        experiments.update(&mut app, |experiments, ctx| {
            experiments.apply_latest_state(vec![ServerExperiment::TestExperiment], ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.0, 1);
        });

        // Redundant experiment state should be a no-op.
        experiments.update(&mut app, |experiments, ctx| {
            experiments.apply_latest_state(vec![ServerExperiment::TestExperiment], ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.0, 1);
        });
    });
}

/// REV-1939: the onboarding offer distinguishes three states, so neither arm
/// and both arms must both fail closed to `Unassigned` rather than guessing.
#[test]
fn choose_how_to_start_arm_resolves_each_assignment_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let cases = [
            (Vec::new(), ChooseHowToStartExperimentArm::Unassigned),
            (
                vec![ServerExperiment::OnboardingChooseHowToStartControl],
                ChooseHowToStartExperimentArm::Control,
            ),
            (
                vec![ServerExperiment::OnboardingChooseHowToStartThreeOptions],
                ChooseHowToStartExperimentArm::Experiment,
            ),
            (
                vec![
                    ServerExperiment::OnboardingChooseHowToStartControl,
                    ServerExperiment::OnboardingChooseHowToStartThreeOptions,
                ],
                ChooseHowToStartExperimentArm::Unassigned,
            ),
        ];

        let experiments =
            app.add_singleton_model(|ctx| ServerExperiments::new_from_cache(vec![], ctx));
        for (state, expected) in cases {
            experiments.update(&mut app, |experiments, ctx| {
                experiments.apply_latest_state(state.clone(), ctx);
            });
            experiments.read(&app, |experiments, _| {
                assert_eq!(
                    experiments.choose_how_to_start_experiment_arm(),
                    expected,
                    "unexpected arm for {state:?}"
                );
            });
        }
    });
}

/// A cached arm must still be superseded by the latest server state, since the
/// offer reads the assignment at exposure time.
#[test]
fn choose_how_to_start_arm_follows_the_latest_state_over_the_cache() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let experiments = app.add_singleton_model(|ctx| {
            ServerExperiments::new_from_cache(
                vec![ServerExperiment::OnboardingChooseHowToStartControl],
                ctx,
            )
        });
        experiments.read(&app, |experiments, _| {
            assert_eq!(
                experiments.choose_how_to_start_experiment_arm(),
                ChooseHowToStartExperimentArm::Control
            );
        });

        experiments.update(&mut app, |experiments, ctx| {
            experiments.apply_latest_state(
                vec![ServerExperiment::OnboardingChooseHowToStartThreeOptions],
                ctx,
            );
        });
        experiments.read(&app, |experiments, _| {
            assert_eq!(
                experiments.choose_how_to_start_experiment_arm(),
                ChooseHowToStartExperimentArm::Experiment
            );
        });
    });
}

/// The GraphQL value, the persisted string, and the client enum must all agree,
/// otherwise a server assignment silently degrades to unassigned.
#[test]
fn choose_how_to_start_arms_round_trip_through_graphql_and_persistence() {
    let cases = [
        (
            Experiment::OnboardingChooseHowToStartControl,
            ServerExperiment::OnboardingChooseHowToStartControl,
            "ONBOARDING_CHOOSE_HOW_TO_START_CONTROL",
        ),
        (
            Experiment::OnboardingChooseHowToStartThreeOptions,
            ServerExperiment::OnboardingChooseHowToStartThreeOptions,
            "ONBOARDING_CHOOSE_HOW_TO_START_THREE_OPTIONS",
        ),
    ];

    for (gql, expected, name) in cases {
        assert_eq!(ServerExperiment::try_from(gql).unwrap(), expected);
        assert_eq!(expected.to_string(), name);
        assert_eq!(
            ServerExperiment::from_string(name.to_string()).unwrap(),
            expected
        );
    }
}

/// Arms the client doesn't know about stay ignored, so a newer server can add
/// values without breaking an older client.
#[test]
fn unknown_experiment_values_are_still_ignored() {
    assert!(ServerExperiment::try_from(Experiment::Other("SOMETHING_NEW".to_string())).is_err());
    assert!(ServerExperiment::from_string("SOMETHING_NEW".to_string()).is_err());
}
