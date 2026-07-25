//! Unit tests for the router warping indicator resolver (APP-4978).
//!
//! These cover the pure resolver seam ([`super::resolve_router_warping`],
//! [`super::classify_router`], [`super::ModelInfoSnapshot`]) with synthetic
//! model ids and output metadata, avoiding a live server or `AppContext`.
//! The live `resolve_router_warping_for_exchange` wrapper is exercised
//! end-to-end by the repo's integration/visual checks; here we lock down the
//! classification, display-label, stale-data, link-target, and feature-gate
//! behavior deterministically.

use std::path::PathBuf;

use super::{
    ModelInfoSnapshot, RouterConfigLink, RouterKind, RouterWarpingResolution, classify_router,
    resolve_router_warping,
};
use crate::ai::custom_model_routers::{CLOUD_CUSTOM_ROUTER_PREFIX, LOCAL_CUSTOM_ROUTER_PREFIX};

fn info(display_name: &str, model_id: &str) -> ModelInfoSnapshot {
    ModelInfoSnapshot {
        display_name: display_name.to_string(),
        model_id: model_id.to_string(),
    }
}

#[test]
fn classify_router_distinguishes_local_cloud_builtin_and_direct() {
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    assert_eq!(classify_router(Some(&local)), Some(RouterKind::CustomLocal));
    assert_eq!(classify_router(Some(&cloud)), Some(RouterKind::CustomCloud));
    // Built-in auto routers.
    assert_eq!(classify_router(Some("auto")), Some(RouterKind::BuiltInAuto));
    assert_eq!(
        classify_router(Some("auto-fast")),
        Some(RouterKind::BuiltInAuto)
    );
    assert_eq!(
        classify_router(Some("cli-agent-auto")),
        Some(RouterKind::BuiltInAuto)
    );
    assert_eq!(
        classify_router(Some("computer-use-agent-auto")),
        Some(RouterKind::BuiltInAuto)
    );
    // Direct (non-router) model ids are ineligible.
    assert_eq!(classify_router(Some("claude-sonnet-4-5")), None);
    assert_eq!(classify_router(None), None);
    // Whitespace is tolerated.
    assert_eq!(
        classify_router(Some("  auto  ")),
        Some(RouterKind::BuiltInAuto)
    );
}

#[test]
fn model_info_snapshot_display_label_prefers_display_name_then_model_id() {
    assert_eq!(
        info("Claude Sonnet", "claude-sonnet-4-5").display_label(),
        Some("Claude Sonnet")
    );
    // Empty display name falls back to the model id.
    assert_eq!(
        info("", "claude-sonnet-4-5").display_label(),
        Some("claude-sonnet-4-5")
    );
    // Both empty -> no label (caller keeps `Warping...`).
    assert_eq!(info("", "").display_label(), None);
}

#[test]
fn resolve_router_warping_flag_disabled_returns_none_even_for_routers() {
    // The new flag off => no router display, regardless of inputs. This is the
    // feature-gate regression: with the flag disabled the existing fallback
    // messaging (governed independently by FallbackModelLoadOutputMessaging)
    // is the only source of warping text.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    assert!(
        resolve_router_warping(
            false,
            Some(&local),
            Some(info("Claude Sonnet", "claude-sonnet-4-5")),
            None,
            false,
            None,
            None,
        )
        .is_none()
    );
}

#[test]
fn resolve_router_warping_direct_model_returns_none() {
    // A direct (non-router) selected model never produces router display, even
    // with the flag on and a resolved model name available. Non-router turns
    // keep the existing implicit `Warping...` text.
    assert!(
        resolve_router_warping(
            true,
            Some("claude-sonnet-4-5"),
            Some(info("Claude Sonnet", "claude-sonnet-4-5")),
            None,
            false,
            None,
            None,
        )
        .is_none()
    );
    assert!(resolve_router_warping(true, None, Some(info("X", "x")), false, None, None).is_none());
}

#[test]
fn resolve_router_warping_builtin_auto_shows_model_without_link() {
    let res = resolve_router_warping(
        true,
        Some("auto"),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        None,
    )
    .expect("builtin auto with a resolved model should resolve");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(res.link, RouterConfigLink::None);
}

#[test]
fn resolve_router_warping_builtin_auto_empty_display_falls_back_to_model_id() {
    let res = resolve_router_warping(
        true,
        Some("auto"),
        Some(info("", "claude-haiku")),
        None,
        false,
        None,
        None,
    )
    .expect("builtin auto with a model id fallback should resolve");
    assert_eq!(res.label, "Warping with claude-haiku.");
    assert_eq!(res.link, RouterConfigLink::None);
}

#[test]
fn resolve_router_warping_missing_model_info_returns_none() {
    // Before ModelUsed arrives, there's nothing to label; the indicator stays
    // on the safe default `Warping...` text.
    assert!(resolve_router_warping(true, Some("auto"), None, None, false, None, None).is_none());
}

#[test]
fn resolve_router_warping_local_custom_with_source_path_links_to_file() {
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let path = PathBuf::from("/home/user/.warp/custom_model_routers/my-router.yaml");
    let res = resolve_router_warping(
        true,
        Some(&local),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        Some(&path),
        None,
    )
    .expect("local custom router with a source path should resolve");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(res.link, RouterConfigLink::OpenLocalFile(path));
}

#[test]
fn resolve_router_warping_local_custom_without_source_path_has_no_link() {
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    // A missing source_path produces no link, but the resolved model still
    // shows (criterion: a pathless local router renders no broken link).
    let res = resolve_router_warping(
        true,
        Some(&local),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        None,
    )
    .expect("local custom router should still resolve without a source path");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(res.link, RouterConfigLink::None);
}

#[test]
fn resolve_router_warping_cloud_custom_links_to_settings_with_router_name() {
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    let res = resolve_router_warping(
        true,
        Some(&cloud),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        Some("Team Router"),
    )
    .expect("cloud custom router should resolve");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(
        res.link,
        RouterConfigLink::OpenCloudSettings {
            search_query: "Team Router".to_string(),
        }
    );
}

#[test]
fn resolve_router_warping_cloud_custom_without_query_falls_back_to_id() {
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    // No display-name query supplied -> fall back to the config_key id so the
    // settings search is still deterministic.
    let res = resolve_router_warping(
        true,
        Some(&cloud),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        None,
    )
    .expect("cloud custom router should resolve even without a query");
    assert_eq!(
        res.link,
        RouterConfigLink::OpenCloudSettings {
            search_query: cloud.clone(),
        }
    );
}

#[test]
fn resolve_router_warping_follow_up_may_use_previous_exchange() {
    // An agent-initiated follow-up (not a new user query) may reuse the
    // immediately previous exchange's model info before ModelUsed arrives,
    // mirroring the existing fallback anti-flicker lookback.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let path = PathBuf::from("/r.yaml");
    let res = resolve_router_warping(
        true,
        Some(&local),
        None, // current exchange has no model info yet
        Some(info("Claude Haiku", "claude-haiku")),
        false, // not a new user query
        Some(&path),
        None,
    )
    .expect("follow-up should fall back to the previous exchange");
    assert_eq!(res.label, "Warping with Claude Haiku.");
    assert_eq!(res.link, RouterConfigLink::OpenLocalFile(path));
}

#[test]
fn resolve_router_warping_new_user_query_never_uses_previous_exchange() {
    // A fresh user query must never display stale model info from an earlier
    // exchange, even if the previous exchange carried a resolved model.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    assert!(
        resolve_router_warping(
            true,
            Some(&local),
            None,
            Some(info("Claude Haiku", "claude-haiku")),
            true, // new user query
            None,
            None,
        )
        .is_none()
    );
}

#[test]
fn router_warping_resolution_link_is_not_displayed_for_builtin_auto() {
    // Sanity: built-in auto never carries a config link, so the footer never
    // renders a "Configure router" affordance for it (criterion 4).
    let res = resolve_router_warping(
        true,
        Some("auto"),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        // Even if a stray local path were supplied, built-in auto ignores it.
        Some(&PathBuf::from("/x.yaml")),
        None,
    )
    .expect("builtin auto should resolve");
    assert_eq!(res.link, RouterConfigLink::None);
    // Confirm the resolution type carries the expected shape for renderers.
    let RouterWarpingResolution { label, link } = res;
    assert!(label.starts_with("Warping with "));
    assert_eq!(link, RouterConfigLink::None);
}
