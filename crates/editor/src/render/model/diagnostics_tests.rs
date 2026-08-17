use warpui_core::App;

use super::*;
use crate::render::model::RenderState;
use crate::render::model::test_utils::TEST_STYLES;

#[test]
fn a_live_model_is_counted_with_its_item_count() {
    App::test((), |mut app| async move {
        let render_state = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));

        let stats = app.read(live_render_state_stats);

        // A fresh pixel-mode model holds the trailing-newline block, so it lands in the bucket
        // above zero rather than in the empty one.
        assert_eq!(stats.live_pixel_models, 1);
        assert_eq!(stats.live_char_cell_models, 0);
        assert_eq!(stats.total_items, 1);
        assert_eq!(stats.largest_model_items, 1);
        assert_eq!(stats.models_by_item_count[0], 0);
        assert_eq!(stats.models_by_item_count[1], 1);
        assert_eq!(stats.models_above_largest_bucket, 0);

        drop(render_state);
    });
}

#[test]
fn a_dropped_model_is_pruned_from_the_registry() {
    App::test((), |mut app| async move {
        let render_state = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));
        assert_eq!(app.read(live_render_state_stats).live_pixel_models, 1);

        drop(render_state);

        assert_eq!(
            app.read(live_render_state_stats).live_pixel_models,
            0,
            "a model that has been dropped must not be counted as live"
        );
    });
}

#[test]
fn larger_trees_land_in_larger_buckets() {
    App::test((), |mut app| async move {
        // 12 items exceeds the 10-item bucket, so this model is counted one bucket higher than a
        // model holding a single block.
        let render_state = app.add_model(|ctx| {
            let mut render_state = RenderState::new(TEST_STYLES, false, None, ctx);
            let mut content = sum_tree::SumTree::new();
            for _ in 0..12 {
                content.push(crate::render::model::test_utils::mock_paragraph(24., 1., 5));
            }
            render_state.set_content(content);
            render_state
        });

        let stats = app.read(live_render_state_stats);

        assert_eq!(stats.models_by_item_count[1], 0);
        assert_eq!(stats.models_by_item_count[2], 1);
        assert!(
            stats.largest_model_items >= 12,
            "expected at least the 12 pushed items, got {stats:?}"
        );

        drop(render_state);
    });
}
