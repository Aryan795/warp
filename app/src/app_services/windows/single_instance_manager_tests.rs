use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use warpui::r#async::block_on;
use warpui::r#async::executor::Background;

use super::{InstanceRole, claim_instance};
use crate::app_services::windows::service_impl::connect_to_sole_running_instance;

/// Names unique to this process and call, so a test never collides with another test or with a
/// Warp instance running on the same machine.
fn unique_object_names() -> (String, String) {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    let id = format!(
        "{}_{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    (
        format!("Local\\WarpTest{id}_SingleInstance"),
        format!("WarpTest{id}_URI_CHANNEL"),
    )
}

/// Connects the way a redirected launch does, so the probe exercises the real retry path rather
/// than a simplified stand-in.
fn listener_is_reachable(pipe_name: &str) -> bool {
    let executor = Arc::new(Background::new(1, |_| "uri-pipe-probe".to_owned()));
    block_on(connect_to_sole_running_instance(pipe_name, executor)).is_ok()
}

/// The contract the claim exists to provide: a launch is only ever told that another instance owns
/// the claim when that instance already has a listener the launch can hand off to. Getting this
/// wrong is what let a second launch fail its connect and start a duplicate instance.
#[test]
fn a_secondary_claim_always_has_a_listener_to_reach() {
    let (mutex_name, pipe_name) = unique_object_names();

    let sole = claim_instance(&mutex_name, &pipe_name).expect("first claim should succeed");
    assert!(
        matches!(sole, InstanceRole::Sole(_)),
        "the first claim on unused names should take the role"
    );

    let secondary = claim_instance(&mutex_name, &pipe_name).expect("second claim should succeed");
    assert!(
        matches!(secondary, InstanceRole::Secondary),
        "a second claim should defer to the instance holding the role"
    );
    assert!(
        listener_is_reachable(&pipe_name),
        "being told another instance owns the claim must imply that instance is reachable"
    );

    drop(sole);
}

/// A process that cannot listen must not leave a claim behind, or the next launch would defer to
/// something it can never reach.
#[test]
fn a_process_that_cannot_listen_leaves_no_claim() {
    let (mutex_name, pipe_name) = unique_object_names();

    let sole = claim_instance(&mutex_name, &pipe_name).expect("first claim should succeed");
    assert!(matches!(sole, InstanceRole::Sole(_)));

    // A distinct mutex with the pipe already taken is the shape of a process whose listener could
    // not be created.
    let (unclaimed_mutex_name, _) = unique_object_names();
    let undiscoverable = claim_instance(&unclaimed_mutex_name, &pipe_name)
        .expect("claiming without a listener should not fail");
    assert!(
        matches!(undiscoverable, InstanceRole::Undiscoverable),
        "failing to listen with no other instance present should not take the role"
    );
    assert!(
        !super::mutex_exists(&unclaimed_mutex_name),
        "a process that cannot listen must leave the mutex untouched"
    );

    drop(sole);
}
