use std::time::Duration;

use instant::Instant;
use warp_editor::content::buffer::{Buffer, BufferSnapshot};
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::IndentBehavior;
use warpui_core::App;
use warpui_core::color::ColorU;

use super::*;

/// Dense, deeply nested, syntactically-invalid SQL: unmatched parens force
/// tree-sitter's error-recovery machinery (`ts_parser__recover`) to repeatedly
/// fork and merge stack versions. `repeat` controls the input size; the caller
/// picks a value that stays under [`MAX_PARSE_BYTES`] so the test exercises
/// [`PARSE_BUDGET`] rather than the cheap size guard.
fn build_pathological_sql(repeat: usize) -> String {
    let mut source = String::from("SELECT * FROM t WHERE ");
    for _ in 0..repeat {
        source.push_str("(a = b AND (c OR (d = (");
    }
    source
}

fn test_color_map() -> ColorMap {
    let black = ColorU {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    ColorMap {
        keyword_color: black,
        function_color: black,
        string_color: black,
        type_color: black,
        number_color: black,
        comment_color: black,
        property_color: black,
        tag_color: black,
    }
}

#[test]
fn test_parse_exceeding_budget_falls_back_instead_of_completing() {
    let language = languages::language_by_name("sql").expect("sql language should be registered");
    // Empirically (see PARSE_BUDGET doc comment) this takes >1s unbounded, well
    // under MAX_PARSE_BYTES.
    let text_content = build_pathological_sql(80_000);
    assert!(
        text_content.len() < MAX_PARSE_BYTES,
        "test input must stay under the size cap to actually exercise PARSE_BUDGET rather than MAX_PARSE_BYTES"
    );

    let outcome = warpui_core::r#async::block_on(async {
        SyntaxTreeState::parse_text(
            BufferSnapshot::from_plain_text(&text_content),
            None,
            &language,
        )
        .await
    });

    assert!(
        matches!(outcome, ParseOutcome::BudgetExceeded),
        "a dense-error parse well within MAX_PARSE_BYTES should trip PARSE_BUDGET instead of running to completion"
    );
}

#[test]
fn test_parse_skips_oversized_buffer_without_attempting_to_parse() {
    let language = languages::language_by_name("sql").expect("sql language should be registered");
    let text_content = "a".repeat(MAX_PARSE_BYTES + 1);

    let outcome = warpui_core::r#async::block_on(async {
        SyntaxTreeState::parse_text(
            BufferSnapshot::from_plain_text(&text_content),
            None,
            &language,
        )
        .await
    });

    assert!(matches!(outcome, ParseOutcome::TooLarge));
}

/// Once a buffer has tripped [`PARSE_BUDGET`], further edits must not re-attempt a
/// full parse (that would re-burn the budget on every keystroke instead of
/// degrading gracefully). This simulates the post-trip state directly, since the
/// budget trip itself is already covered by
/// `test_parse_exceeding_budget_falls_back_instead_of_completing`.
#[test]
fn test_budget_exceeded_latch_skips_reparse_on_next_edit() {
    App::test((), |mut app| async move {
        let language =
            languages::language_by_name("sql").expect("sql language should be registered");

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            *buffer = Buffer::from_plain_text(
                "SELECT 1;",
                None,
                Box::new(|_, _| IndentBehavior::Ignore),
                selection,
                ctx,
            );
        });

        let buffer_version = buffer_handle.read(&app, |buffer, _| buffer.buffer_version());
        let buffer_snapshot = buffer_handle.read(&app, |buffer, _| buffer.buffer_snapshot());

        let syntax_tree_handle = app.add_model(|_| {
            let mut state =
                SyntaxTreeState::new(buffer_handle.downgrade(), buffer_version, test_color_map());
            state.set_language(language);
            state
        });

        // Simulate a prior edit having already tripped the budget.
        syntax_tree_handle.update(&mut app, |state, _ctx| {
            state.parse_budget_exceeded = true;
        });

        let elapsed = {
            let start = Instant::now();
            syntax_tree_handle.update(&mut app, |state, ctx| {
                state.update_internal_state_with_delta(&[], buffer_version, buffer_snapshot, ctx);
            });
            start.elapsed()
        };

        // If the latch didn't short-circuit, this would spawn another tree-sitter
        // parse; for a pathological buffer that means re-spending PARSE_BUDGET on
        // every keystroke instead of staying fast.
        assert!(
            elapsed < Duration::from_millis(100),
            "a latched buffer should skip parsing entirely rather than re-attempt it; took {elapsed:?}"
        );

        syntax_tree_handle.read(&app, |state, _ctx| {
            assert!(state.parse_budget_exceeded, "latch should remain set");
        });
    });
}
