use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use crate::presenter::{COMMITTED_POSITION_FRAME_LIFETIME, PositionCache};

fn rect(size: f32) -> RectF {
    RectF::new(Vector2F::zero(), Vector2F::new(size, size))
}

/// Simulates a frame: the per-frame reset followed by one namespace in which every
/// painted element re-caches its position.
fn paint_frame(position_cache: &mut PositionCache, painted_position_ids: &[&str]) {
    position_cache.clear_single_frame_positions();
    position_cache.start();
    for position_id in painted_position_ids {
        position_cache.cache_position_indefinitely((*position_id).to_string(), rect(100.0));
    }
    position_cache.end();
}

#[test]
fn test_position_cache_caching() {
    let mut position_cache = PositionCache::new();
    position_cache.start();

    position_cache.cache_position_indefinitely(
        "position_1".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(100.0, 100.0)),
    );
    position_cache.cache_position_for_one_frame(
        "position_2".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(50.0, 50.0)),
    );

    position_cache.start();
    position_cache.cache_position_indefinitely(
        "position_1".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(25.0, 25.0)),
    );
    position_cache.cache_position_indefinitely(
        "position_2".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(10.0, 10.0)),
    );
    position_cache.cache_position_for_one_frame(
        "position_3".to_string(),
        RectF::new(Vector2F::zero(), Vector2F::new(5.0, 5.0)),
    );
    assert_eq!(position_cache.get_position("position_1"), None);

    position_cache.end();
    assert_eq!(
        position_cache.get_position("position_1"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(25.0, 25.0)))
    );
    assert_eq!(
        position_cache.get_position("position_2"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(10.0, 10.0)))
    );
    assert_eq!(
        position_cache.get_position("position_3"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(5.0, 5.0)))
    );

    position_cache.end();
    assert_eq!(
        position_cache.get_position("position_1"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(100.0, 100.0)))
    );
    assert_eq!(
        position_cache.get_position("position_2"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(50.0, 50.0)))
    );
    assert_eq!(
        position_cache.get_position("position_3"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(5.0, 5.0)))
    );

    position_cache.clear_single_frame_positions();
    assert_eq!(
        position_cache.get_position("position_1"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(100.0, 100.0)))
    );
    assert_eq!(
        position_cache.get_position("position_2"),
        Some(RectF::new(Vector2F::zero(), Vector2F::new(50.0, 50.0)))
    );
    assert_eq!(position_cache.get_position("position_3"), None);

    position_cache.clear_position("position_1");
    assert_eq!(position_cache.get_position("position_1"), None);
}

#[test]
fn test_committed_positions_survive_a_brief_gap_in_painting() {
    let mut position_cache = PositionCache::new();
    paint_frame(&mut position_cache, &["transiently_hidden"]);

    for _ in 0..(COMMITTED_POSITION_FRAME_LIFETIME - 1) {
        paint_frame(&mut position_cache, &[]);
    }

    assert_eq!(
        position_cache.get_position("transiently_hidden"),
        Some(rect(100.0))
    );
}

#[test]
fn test_committed_positions_expire_once_their_element_stops_painting() {
    let mut position_cache = PositionCache::new();

    for frame in 0..=COMMITTED_POSITION_FRAME_LIFETIME {
        let painted: &[&str] = if frame == 0 {
            &["painted_every_frame", "painted_once"]
        } else {
            &["painted_every_frame"]
        };
        paint_frame(&mut position_cache, painted);
    }

    assert_eq!(
        position_cache.get_position("painted_every_frame"),
        Some(rect(100.0))
    );
    assert_eq!(position_cache.get_position("painted_once"), None);
    assert_eq!(position_cache.committed_position_count(), 1);
}
