use std::cell::RefCell;
use std::rc::Rc;

use warp::appearance::Appearance;
use warpui::event::ModifiersState;
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, EntityIdMap};

use super::{
    InlineMenuScrollFn, TuiInlineMenuElement, TuiInlineMenuHeader, TuiInlineMenuListState,
    TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus,
    TuiInlineMenuTab, render_inline_menu,
};
use crate::tui_builder::TuiUiBuilder;

/// Lays out `element` at `area` so that content sizes are populated for
/// subsequent render/dispatch calls.
fn layout_element(element: &mut dyn TuiElement, area: TuiRect, ctx: &AppContext) {
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(
        TuiConstraint::tight(TuiSize::new(area.width, area.height)),
        &mut lctx,
        ctx,
    );
}

/// Renders `element` into a fresh buffer then dispatches `event`, recording
/// element origins in the scene so hit-tests work correctly.
/// Mirrors `transcript_view_tests.rs::dispatch_event`. Layout must have run.
fn render_and_dispatch(
    element: &mut dyn TuiElement,
    area: TuiRect,
    event: &TuiEvent,
    ctx: &AppContext,
) -> bool {
    let mut rendered_views = EntityIdMap::default();
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(
            TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
            &mut surface,
            &mut paint_ctx,
        );
    }
    let scene = Rc::new(paint_ctx.scene.clone());
    drop(paint_ctx);
    let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
    event_ctx.set_origin_view(Some(EntityId::new()));
    element.dispatch_event(event, &mut event_ctx, ctx)
}

/// Builds an interactive `TuiInlineMenuElement` for use in unit tests.
/// The closures capture the provided `Rc<RefCell<_>>` cells so the test can
/// observe what index was accepted and what delta the scroll received.
fn make_interactive_element(
    snapshot: TuiInlineMenuSnapshot,
    ctx: &AppContext,
    accepted_index: Rc<RefCell<Option<usize>>>,
    scroll_delta: Rc<RefCell<Option<isize>>>,
) -> TuiInlineMenuElement {
    use warpui_core::elements::MouseStateHandle;
    let on_accept = {
        let accepted_index = Rc::clone(&accepted_index);
        move |index: usize, _: &mut TuiEventContext<'_>, _: &AppContext| {
            *accepted_index.borrow_mut() = Some(index);
        }
    };
    let on_scroll: Box<InlineMenuScrollFn> = {
        let scroll_delta = Rc::clone(&scroll_delta);
        Box::new(
            move |delta: isize, _: &mut TuiEventContext<'_>, _: &AppContext| {
                *scroll_delta.borrow_mut() = Some(delta);
            },
        )
    };
    let item_mouse_states = Rc::new(RefCell::new(
        (0..snapshot.rows.len())
            .map(|_| MouseStateHandle::default())
            .collect::<Vec<_>>(),
    ));
    TuiInlineMenuElement {
        snapshot,
        builder: TuiUiBuilder::from_app(ctx),
        content: None,
        item_mouse_states,
        last_row_titles: Rc::new(RefCell::new(Vec::new())),
        on_accept: Some(Rc::new(on_accept)),
        on_scroll: Some(on_scroll),
    }
}

fn render_at_size(snapshot: TuiInlineMenuSnapshot, width: u16, height: u16) -> Vec<String> {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)),
                TuiRect::new(0, 0, width, height),
                ctx,
            );
            frame.buffer.to_lines()
        })
    })
}

fn render_at_height(snapshot: TuiInlineMenuSnapshot, height: u16) -> Vec<String> {
    render_at_size(snapshot, 50, height)
}
fn render(snapshot: TuiInlineMenuSnapshot) -> Vec<String> {
    render_at_height(snapshot, 12)
}
fn rendered_labels(snapshot: TuiInlineMenuSnapshot, height: u16) -> Vec<String> {
    let mut lines = render_at_height(snapshot, height)
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

fn rows_snapshot(
    row_count: usize,
    selected_index: usize,
    scroll_offset: usize,
    max_visible_rows: usize,
) -> TuiInlineMenuSnapshot {
    TuiInlineMenuSnapshot {
        header: None,
        rows: (0..row_count)
            .map(|index| TuiInlineMenuRow {
                title: format!("Conversation {index}"),
                description: None,
                state_suffix: None,
                is_selectable: true,
                style: TuiInlineMenuRowStyle::Default,
            })
            .collect(),
        selected_index: Some(selected_index),
        scroll_offset,
        max_visible_rows,
        status: None,
    }
}

fn status_snapshot(status: TuiInlineMenuStatus) -> TuiInlineMenuSnapshot {
    TuiInlineMenuSnapshot {
        header: None,
        rows: Vec::new(),
        selected_index: None,
        scroll_offset: 0,
        max_visible_rows: 8,
        status: Some(status),
    }
}

#[test]
fn renders_loading_and_empty_statuses() {
    let loading = render(status_snapshot(TuiInlineMenuStatus::Loading(
        "Loading conversations…".to_owned(),
    )));
    assert!(
        loading
            .iter()
            .any(|line| line.contains("Loading conversations…"))
    );

    let empty = render(status_snapshot(TuiInlineMenuStatus::Empty(
        "No conversations found".to_owned(),
    )));
    assert!(
        empty
            .iter()
            .any(|line| line.contains("No conversations found"))
    );
}

#[test]
fn renders_only_the_visible_row_window() {
    assert_eq!(
        rendered_labels(rows_snapshot(5, 3, 2, 2), 12),
        vec!["Conversation 2", "Conversation 3"]
    );
}

#[test]
fn fitting_rows_render_without_overflow_indicators() {
    assert_eq!(
        rendered_labels(rows_snapshot(3, 1, 0, 4), 4),
        vec!["Conversation 0", "Conversation 1", "Conversation 2"]
    );
}

#[test]
fn lower_overflow_renders_a_down_arrow_as_the_last_row() {
    assert_eq!(
        rendered_labels(rows_snapshot(5, 1, 0, 4), 4),
        vec!["Conversation 0", "Conversation 1", "Conversation 2", "↓"]
    );
}
#[test]
fn multiline_conversation_title_is_ellipsized_without_hiding_lower_overflow() {
    let mut snapshot = rows_snapshot(5, 0, 0, 4);
    snapshot.rows[0].title = "Conversation 0\ncontinued title".to_owned();

    assert_eq!(
        rendered_labels(snapshot, 4),
        vec!["Conversation 0...", "Conversation 1", "Conversation 2", "↓"]
    );
}

#[test]
fn upper_overflow_renders_an_up_arrow_as_the_first_row() {
    assert_eq!(
        rendered_labels(rows_snapshot(5, 4, 3, 4), 4),
        vec!["↑", "Conversation 2", "Conversation 3", "Conversation 4"]
    );
}

#[test]
fn overflow_in_both_directions_renders_both_arrows_and_keeps_selection_visible() {
    assert_eq!(
        rendered_labels(rows_snapshot(7, 4, 0, 5), 5),
        vec![
            "↑",
            "Conversation 2",
            "Conversation 3",
            "Conversation 4",
            "↓"
        ]
    );
}

#[test]
fn short_viewport_prioritizes_three_real_rows_over_scroll_indicators() {
    assert_eq!(
        rendered_labels(rows_snapshot(7, 3, 0, 4), 4),
        vec![
            "Conversation 0",
            "Conversation 1",
            "Conversation 2",
            "Conversation 3"
        ]
    );
}

#[test]
fn conversation_like_snapshot_reuses_header_tabs_rows_and_selection() {
    let lines = render(TuiInlineMenuSnapshot {
        header: Some(TuiInlineMenuHeader {
            title: Some("Conversations".to_owned()),
            tabs: vec![
                TuiInlineMenuTab {
                    label: "All".to_owned(),
                    is_selected: true,
                },
                TuiInlineMenuTab {
                    label: "Pinned".to_owned(),
                    is_selected: false,
                },
            ],
        }),
        rows: vec![
            TuiInlineMenuRow {
                title: "Current project".to_owned(),
                description: Some("2 minutes ago".to_owned()),
                state_suffix: None,
                is_selectable: true,
                style: TuiInlineMenuRowStyle::Default,
            },
            TuiInlineMenuRow {
                title: "Archived".to_owned(),
                description: None,
                state_suffix: None,
                is_selectable: false,
                style: TuiInlineMenuRowStyle::Default,
            },
        ],
        selected_index: Some(0),
        scroll_offset: 0,
        max_visible_rows: 8,
        status: None,
    });
    let rendered = lines.join("\n");
    assert!(rendered.contains("Conversations"));
    assert!(rendered.contains("[All]  Pinned"));
    assert!(!rendered.chars().any(|glyph| "┌┐└┘─│".contains(glyph)));
    assert!(rendered.contains("Current project  2 minutes ago"));
    assert!(rendered.contains("Archived"));
}

#[test]
fn conversation_like_snapshot_keeps_selection_visible_within_production_height() {
    let lines = render_at_height(
        TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Conversations".to_owned()),
                tabs: vec![
                    TuiInlineMenuTab {
                        label: "All".to_owned(),
                        is_selected: true,
                    },
                    TuiInlineMenuTab {
                        label: "Pinned".to_owned(),
                        is_selected: false,
                    },
                ],
            }),
            rows: (0..8)
                .map(|index| TuiInlineMenuRow {
                    title: format!("Conversation {index}"),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: Some(7),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        10,
    );

    assert_eq!(lines.len(), 10);
    let rendered = lines.join("\n");
    assert!(rendered.contains("Conversations"));
    assert!(rendered.contains("[All]  Pinned"));
    assert!(rendered.contains("Conversation 0"));
    assert!(rendered.contains("Conversation 1"));
    assert!(rendered.contains("Conversation 2"));
    assert!(rendered.contains("Conversation 7"));
}

#[test]
fn slash_command_rows_match_figma_layout_and_colors() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let snapshot = TuiInlineMenuSnapshot {
                header: None,
                rows: vec![
                    TuiInlineMenuRow {
                        title: "/agent".to_owned(),
                        description: Some("Start a new agent conversation".to_owned()),
                        state_suffix: Some("(currently on)".to_owned()),
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::InlineMenuItem,
                    },
                    TuiInlineMenuRow {
                        title: "/plan".to_owned(),
                        description: Some("Create a plan".to_owned()),
                        state_suffix: Some("(currently off)".to_owned()),
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::InlineMenuItem,
                    },
                ],
                selected_index: Some(0),
                scroll_offset: 0,
                max_visible_rows: 8,
                status: None,
            };
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &builder),
                TuiRect::new(0, 0, 80, 2),
                ctx,
            );
            let lines = frame.buffer.to_lines();

            assert!(lines[0].starts_with(
                "/agent                       Start a new agent conversation (currently on)"
            ));
            assert!(lines[1].starts_with("/plan                        Create"));
            assert!(
                !lines
                    .iter()
                    .any(|line| line.chars().any(|glyph| "┌┐└┘─│".contains(glyph)))
            );
            assert_eq!(
                frame.buffer[(0, 0)].bg,
                builder.slash_command_selection_background()
            );
            assert_eq!(frame.buffer[(0, 0)].bg, Color::Rgb(208, 209, 254));
            assert_eq!(
                frame.buffer[(0, 0)].fg,
                builder
                    .slash_command_selection_text_style()
                    .fg
                    .expect("selected slash-command text has a foreground")
            );
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                builder
                    .slash_command_text_style()
                    .fg
                    .expect("slash-command text has a foreground")
            );
            assert_eq!(
                frame.buffer[(29, 1)].fg,
                builder
                    .primary_text_style()
                    .fg
                    .expect("slash-command descriptions use primary text")
            );
            let suffix_column = lines[0]
                .find("(currently on)")
                .expect("state suffix should render");
            assert_eq!(
                frame.buffer[(u16::try_from(suffix_column).unwrap(), 0)].fg,
                builder
                    .slash_command_selection_state_suffix_style()
                    .fg
                    .expect("selected state suffix should use muted theme green")
            );
            let unselected_suffix_column = lines[1]
                .find("(currently off)")
                .expect("unselected state suffix should render");
            assert_eq!(
                frame.buffer[(u16::try_from(unselected_suffix_column).unwrap(), 1)].fg,
                builder
                    .success_glyph_style()
                    .fg
                    .expect("unselected state suffix should use theme green")
            );
            assert_eq!(
                frame.buffer[(u16::try_from(unselected_suffix_column).unwrap(), 1)].fg,
                Color::Rgb(180, 250, 114)
            );
        });
    });
}

#[test]
fn long_slash_command_titles_are_ellipsized_before_the_description() {
    let lines = render(TuiInlineMenuSnapshot {
        header: None,
        rows: vec![TuiInlineMenuRow {
            title: "/respond-to-pr-comments-in-blocklist".to_owned(),
            description: Some("Walk users through PR review comments".to_owned()),
            state_suffix: None,
            is_selectable: true,
            style: TuiInlineMenuRowStyle::InlineMenuItem,
        }],
        selected_index: Some(0),
        scroll_offset: 0,
        max_visible_rows: 8,
        status: None,
    });

    assert!(lines[0].starts_with("/respond-to-pr-comments-i... Walk users"));
}

#[test]
fn wide_slash_command_rows_expand_to_show_long_titles() {
    let lines = render_at_size(
        TuiInlineMenuSnapshot {
            header: None,
            rows: vec![TuiInlineMenuRow {
                title: "/respond-to-pr-comments-in-blocklist".to_owned(),
                description: Some("Walk users through PR review comments".to_owned()),
                state_suffix: None,
                is_selectable: true,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            }],
            selected_index: Some(0),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        80,
        1,
    );

    assert!(
        lines[0].starts_with(
            "/respond-to-pr-comments-in-blocklist Walk users through PR review comments"
        )
    );
}

#[test]
fn boundary_width_preserves_useful_title_and_description_columns() {
    let lines = render_at_size(
        TuiInlineMenuSnapshot {
            header: None,
            rows: vec![TuiInlineMenuRow {
                title: "/agent".to_owned(),
                description: Some("Start a new agent conversation".to_owned()),
                state_suffix: None,
                is_selectable: true,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            }],
            selected_index: Some(0),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        20,
        1,
    );

    assert!(lines[0].starts_with("/agent  Start a new"));
}

#[test]
fn narrow_slash_command_rows_use_the_full_width_for_titles() {
    let lines = render_at_size(
        TuiInlineMenuSnapshot {
            header: None,
            rows: vec![TuiInlineMenuRow {
                title: "/12345678901234567890".to_owned(),
                description: Some("Description hidden at narrow widths".to_owned()),
                state_suffix: None,
                is_selectable: true,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            }],
            selected_index: Some(0),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        19,
        1,
    );

    assert_eq!(lines[0], "/123456789012345...");
}

#[test]
fn list_select_absolute_sets_selection_and_scrolls_to_keep_visible() {
    // 5 rows, max 2 visible. Select row 3 (index 3, 0-based) absolutely.
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec![(); 5], false, Some(0), 2, |_| true);

    list.select_absolute(3, 2, |_| true);
    assert_eq!(list.selected_index(), Some(3));
    // With 2 visible rows the viewport starts at scroll_offset=2 so row 3
    // lands in the [2, 4) window.
    assert_eq!(
        list.scroll_offset(),
        2,
        "scroll offset should make row 3 visible"
    );
}

#[test]
fn list_select_absolute_ignores_out_of_bounds_index() {
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec![(); 3], false, Some(1), 3, |_| true);

    list.select_absolute(10, 3, |_| true); // out of bounds — no change
    assert_eq!(list.selected_index(), Some(1));
}

#[test]
fn list_select_absolute_skips_non_selectable_row() {
    // Row 0 is selectable, row 1 is not, row 2 is selectable.
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec![true, false, true], false, Some(0), 3, |row| *row);

    // Clicking the non-selectable row must leave the selection unchanged.
    list.select_absolute(1, 3, |row| *row);
    assert_eq!(list.selected_index(), Some(0));

    // Clicking a selectable row should work normally.
    list.select_absolute(2, 3, |row| *row);
    assert_eq!(list.selected_index(), Some(2));
}

#[test]
fn list_scroll_by_moves_offset_without_changing_selection() {
    let mut list = TuiInlineMenuListState::default();
    // 8 rows, 3 visible; start at offset 0, selection at row 1.
    list.replace_rows(vec![(); 8], false, Some(1), 3, |_| true);
    assert_eq!(list.selected_index(), Some(1));
    assert_eq!(list.scroll_offset(), 0);

    list.scroll_by(2, 3);
    // Selection unchanged, scroll offset moved forward.
    assert_eq!(list.selected_index(), Some(1));
    assert_eq!(list.scroll_offset(), 2);

    // Negative scroll brings it back.
    list.scroll_by(-1, 3);
    assert_eq!(list.scroll_offset(), 1);

    // Scroll past the end is clamped.
    list.scroll_by(100, 3);
    assert_eq!(list.scroll_offset(), 5); // max is 8 - 3 = 5
}

#[test]
fn shared_list_navigation_wraps_skips_disabled_rows_and_scrolls() {
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec![true, false, true, true], false, Some(0), 2, |row| *row);

    list.select_next(2, |row| *row);
    assert_eq!(list.selected_index(), Some(2));
    assert_eq!(list.scroll_offset(), 1);

    list.select_next(2, |row| *row);
    assert_eq!(list.selected_index(), Some(3));
    assert_eq!(list.scroll_offset(), 2);

    list.select_next(2, |row| *row);
    assert_eq!(list.selected_index(), Some(0));
    assert_eq!(list.scroll_offset(), 0);

    list.select_previous(2, |row| *row);
    assert_eq!(list.selected_index(), Some(3));
    assert_eq!(list.scroll_offset(), 2);
}

#[test]
fn shared_list_reserves_space_for_scroll_indicators() {
    let mut list = TuiInlineMenuListState::default();

    list.replace_rows(vec![(); 11], false, Some(10), 10, |_| true);

    assert_eq!(list.selected_index(), Some(10));
    assert_eq!(list.scroll_offset(), 2);
}

#[test]
fn shared_list_preserves_ready_rows_while_a_mixer_query_loads() {
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec!["ready"], false, Some(0), 2, |_| true);

    let update = list.reconcile_mixer_rows(vec!["pending"], true, 2, |_| true);

    assert_eq!(
        update,
        warp_search_core::inline_menu::InlineMenuResultsUpdate::Loading
    );
    assert_eq!(list.rows(), &["ready"]);
    assert_eq!(list.selected_index(), Some(0));
    assert!(list.is_loading());
}

// --- Interactive render-path tests ---
// These tests exercise the interactive TuiInlineMenuElement::dispatch_event
// branch (TuiHoverable click, scroll-wheel, out-of-bounds scroll), which the
// non-interactive render_inline_menu helper never touches.

#[test]
fn interactive_menu_click_on_selectable_row_fires_on_accept() {
    // 3 selectable rows rendered in a 50x3 area; click lands on row 1 (y=1).
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let accepted = Rc::new(RefCell::new(None::<usize>));
            let scroll = Rc::new(RefCell::new(None::<isize>));
            let snapshot = rows_snapshot(3, 0, 0, 5);
            let mut element =
                make_interactive_element(snapshot, ctx, Rc::clone(&accepted), Rc::clone(&scroll));

            let area = TuiRect::new(0, 0, 50, 3);

            // Layout first so that content sizes are known for hit-testing.
            layout_element(&mut element, area, ctx);

            // Simulate LeftMouseDown + LeftMouseUp at row 1 (y=1) to trigger a
            // click. Each render_and_dispatch call re-renders and re-records
            // the scene so element origins remain valid for hit-testing.
            let down = TuiEvent::LeftMouseDown {
                position: (5u16, 1u16).into(),
                modifiers: ModifiersState::default(),
                click_count: 1,
                is_first_mouse: false,
            };
            let up = TuiEvent::LeftMouseUp {
                position: (5u16, 1u16).into(),
                modifiers: ModifiersState::default(),
            };
            render_and_dispatch(&mut element, area, &down, ctx);
            render_and_dispatch(&mut element, area, &up, ctx);

            assert_eq!(
                *accepted.borrow(),
                Some(1),
                "clicking row 1 should fire on_accept with index 1"
            );
            assert_eq!(*scroll.borrow(), None, "no scroll should fire on a click");
        });
    });
}

#[test]
fn interactive_menu_scroll_wheel_calls_on_scroll_with_negated_delta() {
    // Verify that a positive wheel delta (user scrolls down) produces a
    // *negative* delta to the on_scroll callback, matching option_selector
    // and scrollable: positive delta = scroll toward the start of the list.
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let accepted = Rc::new(RefCell::new(None::<usize>));
            let scroll = Rc::new(RefCell::new(None::<isize>));
            let snapshot = rows_snapshot(3, 0, 0, 5);
            let mut element =
                make_interactive_element(snapshot, ctx, Rc::clone(&accepted), Rc::clone(&scroll));

            let area = TuiRect::new(0, 0, 50, 3);
            layout_element(&mut element, area, ctx);

            // Positive y-delta — pointer is inside the element area (y=1).
            let event = TuiEvent::ScrollWheel {
                position: (5u16, 1u16).into(),
                delta: (0, 2),
                precise: false,
                modifiers: ModifiersState::default(),
            };
            let handled = render_and_dispatch(&mut element, area, &event, ctx);

            assert!(handled, "in-bounds scroll must be consumed");
            assert_eq!(
                *scroll.borrow(),
                Some(-2),
                "on_scroll must receive -delta.1 so the viewport scrolls toward the start"
            );
            assert_eq!(*accepted.borrow(), None);
        });
    });
}

#[test]
fn interactive_menu_scroll_wheel_outside_bounds_is_not_handled() {
    // A wheel event whose position lies outside the rendered element area must
    // pass through unhandled so the transcript scrollable beneath the menu can
    // still receive it.
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let accepted = Rc::new(RefCell::new(None::<usize>));
            let scroll = Rc::new(RefCell::new(None::<isize>));
            let snapshot = rows_snapshot(3, 0, 0, 5);
            let mut element =
                make_interactive_element(snapshot, ctx, Rc::clone(&accepted), Rc::clone(&scroll));

            // Element occupies rows 0-2; pointer at y=10 is well outside.
            let area = TuiRect::new(0, 0, 50, 3);
            layout_element(&mut element, area, ctx);

            let event = TuiEvent::ScrollWheel {
                position: (5u16, 10u16).into(),
                delta: (0, 1),
                precise: false,
                modifiers: ModifiersState::default(),
            };
            let handled = render_and_dispatch(&mut element, area, &event, ctx);

            assert!(!handled, "out-of-bounds scroll must not be consumed");
            assert_eq!(*scroll.borrow(), None, "on_scroll must not fire");
        });
    });
}

#[test]
fn interactive_menu_hovered_selectable_row_renders_bold() {
    // Verify that a row whose MouseStateHandle is in hovered state renders
    // with the BOLD modifier, confirming that hover feedback is visible.
    //
    // The hover state is stored on the shared MouseStateHandle and is read
    // during layout (in build_inline_menu). So the flow is:
    //   1. layout → build content (no hover yet)
    //   2. render+dispatch MouseMoved → TuiHoverable sets is_hovered=true on
    //      the shared handle
    //   3. re-layout → build_inline_menu reads is_hovered=true, applies BOLD
    //   4. render to buffer and assert BOLD is present
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let accepted = Rc::new(RefCell::new(None::<usize>));
            let scroll = Rc::new(RefCell::new(None::<isize>));
            // Use 3 rows all selectable; row 0 is initially selected.
            let snapshot = rows_snapshot(3, 0, 0, 5);
            let mut element =
                make_interactive_element(snapshot, ctx, Rc::clone(&accepted), Rc::clone(&scroll));

            let area = TuiRect::new(0, 0, 50, 3);
            // Step 1: initial layout so origins are available for hit-testing.
            layout_element(&mut element, area, ctx);

            // Step 2: dispatch MouseMoved at row 1 (y=1).
            // render_and_dispatch renders first (recording scene origins) then
            // dispatches, so TuiHoverable can hit-test and set is_hovered.
            let hover_event = TuiEvent::MouseMoved {
                position: (5u16, 1u16).into(),
                modifiers: ModifiersState::default(),
                is_synthetic: false,
            };
            render_and_dispatch(&mut element, area, &hover_event, ctx);

            // Step 3: re-layout so build_inline_menu reads the updated hover
            // state and rebuilds the content tree with BOLD on row 1.
            layout_element(&mut element, area, ctx);

            // Step 4: render to a buffer and inspect the cells.
            let mut rendered_views = EntityIdMap::default();
            let mut buffer = TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(
                    TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
                    &mut surface,
                    &mut paint_ctx,
                );
            }
            // Row 1 should be rendered with the BOLD modifier (hover feedback).
            let cell = &buffer[(0u16, 1u16)];
            assert!(
                cell.modifier.contains(Modifier::BOLD),
                "hovered selectable row must render bold; modifier was {:?}",
                cell.modifier
            );
            // Row 0 should not be bold (it is selected, which uses a different
            // style, not bold; and it is not hovered).
            // Row 2 is not hovered and not selected so also must not be bold.
            assert!(
                !buffer[(0u16, 2u16)].modifier.contains(Modifier::BOLD),
                "non-hovered row 2 must not be bold"
            );
        });
    });
}

#[test]
fn interactive_menu_click_on_non_selectable_row_does_not_fire_on_accept() {
    // A click on a non-selectable row (is_selectable = false) must not call
    // on_accept; the row has no click handler so the click falls through.
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let accepted = Rc::new(RefCell::new(None::<usize>));
            let scroll = Rc::new(RefCell::new(None::<isize>));
            // Build a snapshot with row 1 marked non-selectable.
            let snapshot = TuiInlineMenuSnapshot {
                header: None,
                rows: vec![
                    TuiInlineMenuRow {
                        title: "Selectable".to_owned(),
                        description: None,
                        state_suffix: None,
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::Default,
                    },
                    TuiInlineMenuRow {
                        title: "Non-selectable header".to_owned(),
                        description: None,
                        state_suffix: None,
                        is_selectable: false,
                        style: TuiInlineMenuRowStyle::Default,
                    },
                    TuiInlineMenuRow {
                        title: "Also selectable".to_owned(),
                        description: None,
                        state_suffix: None,
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::Default,
                    },
                ],
                selected_index: Some(0),
                scroll_offset: 0,
                max_visible_rows: 5,
                status: None,
            };
            let mut element =
                make_interactive_element(snapshot, ctx, Rc::clone(&accepted), Rc::clone(&scroll));

            let area = TuiRect::new(0, 0, 50, 3);
            layout_element(&mut element, area, ctx);

            // Click on the non-selectable row at y=1.
            let down = TuiEvent::LeftMouseDown {
                position: (5u16, 1u16).into(),
                modifiers: ModifiersState::default(),
                click_count: 1,
                is_first_mouse: false,
            };
            let up = TuiEvent::LeftMouseUp {
                position: (5u16, 1u16).into(),
                modifiers: ModifiersState::default(),
            };
            render_and_dispatch(&mut element, area, &down, ctx);
            render_and_dispatch(&mut element, area, &up, ctx);

            assert_eq!(
                *accepted.borrow(),
                None,
                "clicking a non-selectable row must not fire on_accept"
            );
        });
    });
}
