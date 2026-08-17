//! Diagnostics for how many [`RenderState`] content trees are alive, and how large each one is.
//!
//! Heap profiles attribute most of the process's memory to the editor's content trees but cannot
//! tell a few enormous trees apart from very many small ones, so they keep coming back
//! inconclusive (APP-5445). Every live model registers a weak handle here, and the stats come from
//! tree summaries that are maintained anyway: collecting them costs one summary read per live
//! model and never walks a tree.
//!
//! This is diagnostic only. Nothing here affects layout, what is laid out, or when.

use std::cell::RefCell;

use warpui_core::{AppContext, WeakModelHandle};

use super::RenderState;

/// Inclusive upper bounds, in items, of the buckets a model's content tree is counted in. A model
/// falls in the first bucket whose bound it does not exceed; larger trees are counted separately in
/// [`RenderStateStats::models_above_largest_bucket`].
const ITEM_COUNT_BUCKETS: [usize; 6] = [0, 10, 100, 1_000, 10_000, 100_000];

thread_local! {
    /// Every [`RenderState`] built on this thread through a non-test constructor, as a weak handle.
    ///
    /// Thread-local rather than global because reading a model requires an [`AppContext`], which
    /// confines model access to the thread that owns them — so a registry per thread sees exactly
    /// the models a caller on that thread could read, and needs no lock. `WeakModelHandle` is also
    /// not `Sync`, since `RenderState` holds `Cell`s.
    ///
    /// Entries for dropped models are pruned by [`live_render_state_stats`] rather than on drop, so
    /// `RenderState` needs no `Drop` impl — adding one would change move semantics for a type on
    /// the layout hot path, which is not a trade worth making for a diagnostic. Between reads the
    /// registry therefore holds one dead entry per model that has come and gone, each an
    /// `EntityId`.
    static LIVE_RENDER_STATES: RefCell<Vec<WeakModelHandle<RenderState>>> =
        const { RefCell::new(Vec::new()) };
}

/// Record a newly built model so its content tree is visible to [`live_render_state_stats`].
pub(super) fn register(handle: WeakModelHandle<RenderState>) {
    LIVE_RENDER_STATES.with_borrow_mut(|registered| registered.push(handle));
}

/// How many content trees are alive, and how their sizes are distributed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderStateStats {
    /// Live models on the pixel (GUI) layout path. These are the ones that hold a content tree.
    pub live_pixel_models: usize,
    /// Live models on the char-cell (TUI) layout path. These never populate the `SumTree`, so they
    /// are counted here instead of swelling the zero-item bucket and reading as idle editors.
    pub live_char_cell_models: usize,
    /// Items across every live pixel-mode content tree.
    pub total_items: usize,
    /// Items in the largest single pixel-mode content tree.
    pub largest_model_items: usize,
    /// Pixel-mode models per bucket, parallel to [`RenderStateStats::bucket_upper_bounds`].
    pub models_by_item_count: [usize; ITEM_COUNT_BUCKETS.len()],
    /// Pixel-mode models holding more items than the largest bucket's bound.
    pub models_above_largest_bucket: usize,
}

impl RenderStateStats {
    /// The bucket bounds `models_by_item_count` is indexed by, so a caller can label the counts
    /// without restating them.
    pub fn bucket_upper_bounds() -> &'static [usize] {
        &ITEM_COUNT_BUCKETS
    }
}

/// Collect stats for every live [`RenderState`], dropping registry entries whose model is gone.
///
/// Costs one weak-handle upgrade and one root-summary read per live model.
pub fn live_render_state_stats(app: &AppContext) -> RenderStateStats {
    let mut stats = RenderStateStats::default();

    LIVE_RENDER_STATES.with_borrow_mut(|registered| {
        registered.retain(|weak_handle| {
            let Some(handle) = weak_handle.upgrade(app) else {
                return false;
            };

            let render_state = handle.as_ref(app);
            if render_state.char_cell().is_some() {
                stats.live_char_cell_models += 1;
                return true;
            }

            let items = render_state.content_item_count();
            stats.live_pixel_models += 1;
            stats.total_items += items;
            stats.largest_model_items = stats.largest_model_items.max(items);
            match ITEM_COUNT_BUCKETS.iter().position(|bound| items <= *bound) {
                Some(bucket) => stats.models_by_item_count[bucket] += 1,
                None => stats.models_above_largest_bucket += 1,
            }
            true
        });
    });

    stats
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
