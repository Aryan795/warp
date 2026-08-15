//! Tests for the restore-seed scope selection. The listing/link loop and the
//! ancestor-pane-chain reveal are covered in `pane_group/mod_tests.rs`, which
//! owns the mock pane-group harness.

use chrono::Utc;
use warp_core::features::FeatureFlag;

use super::*;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};

fn parent_task_id() -> AmbientAgentTaskId {
    "11111111-1111-1111-1111-111111111111".parse().unwrap()
}

fn task_row(parent_run_id: Option<&str>) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id: parent_task_id(),
        parent_run_id: parent_run_id.map(str::to_string),
        title: "parent".to_string(),
        state: AmbientAgentTaskState::InProgress,
        prompt: String::new(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: None,
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

#[test]
fn root_parent_seeds_subtree_listing() {
    let _multi_level = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    let _unified = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);

    let filter = restore_seed_filter(parent_task_id(), Some(&task_row(None)))
        .expect("scope resolvable from a cached root row");
    assert_eq!(filter.root_run_id, Some(parent_task_id().to_string()));
    assert_eq!(filter.ancestor_run_id, None);
}

#[test]
fn mid_tree_parent_seeds_direct_children_listing() {
    let _multi_level = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    let _unified = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);

    let filter = restore_seed_filter(
        parent_task_id(),
        Some(&task_row(Some("22222222-2222-2222-2222-222222222222"))),
    )
    .expect("scope resolvable from a cached mid-tree row");
    assert_eq!(filter.ancestor_run_id, Some(parent_task_id().to_string()));
    assert_eq!(filter.root_run_id, None);
}

#[test]
fn unknown_root_ness_defers_the_seed() {
    // Guessing the scope could permanently miss grandchildren; the seed
    // stays pending until the parent's row is cached.
    let _multi_level = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    let _unified = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);

    assert!(restore_seed_filter(parent_task_id(), None).is_none());
}

#[test]
fn subtree_scope_disabled_always_seeds_direct_children() {
    let _multi_level = FeatureFlag::MultiLevelOrchestration.override_enabled(false);
    let _unified = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);

    // Even a cached root row keeps the legacy direct-children listing, and
    // an uncached row does not defer.
    for cached in [Some(task_row(None)), None] {
        let filter = restore_seed_filter(parent_task_id(), cached.as_ref())
            .expect("legacy scope never defers");
        assert!(filter.ancestor_run_id.is_some());
        assert_eq!(filter.root_run_id, None);
    }
}
