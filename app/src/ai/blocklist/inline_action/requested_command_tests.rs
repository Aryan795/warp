//! Unit tests for format_command_text in requested_command.rs

use pathfinder_color::ColorU;
use string_offset::CharOffset;
use warp_completer::completer::{Description, SuggestionType, TopLevelCommandCaseSensitivity};
use warp_completer::meta::Span;
use warp_completer::{ParsedTokenData, ParsedTokensSnapshot};
use warp_core::ui::theme::{AnsiColor, AnsiColors};

use super::{
    command_highlight_color_ranges, format_command_text, header_highlight_ranges,
    mcp_blocked_title_text, mcp_viewing_detail_title_text,
};

/// Distinct-per-channel `AnsiColors` fixture so tests can assert exact colors without depending
/// on any real theme.
fn test_terminal_colors() -> AnsiColors {
    AnsiColors::new(
        AnsiColor {
            r: 10,
            g: 10,
            b: 10,
        },
        AnsiColor { r: 20, g: 0, b: 0 },
        AnsiColor { r: 0, g: 30, b: 0 },
        AnsiColor { r: 40, g: 40, b: 0 },
        AnsiColor { r: 0, g: 0, b: 50 },
        AnsiColor { r: 60, g: 0, b: 60 },
        AnsiColor { r: 0, g: 70, b: 70 },
        AnsiColor {
            r: 80,
            g: 80,
            b: 80,
        },
    )
}

fn parsed_token(
    text: &str,
    span: (usize, usize),
    description: Option<SuggestionType>,
) -> ParsedTokenData {
    let span = Span::new(span.0, span.1);
    ParsedTokenData {
        token: warp_completer::meta::Spanned {
            span,
            item: text.to_string(),
        },
        token_index: 0,
        token_description: description.map(|suggestion_type| Description {
            token: warp_completer::meta::Spanned {
                span,
                item: text.to_string(),
            },
            description_text: None,
            suggestion_type,
        }),
    }
}

#[test]
fn single_line_without_newline_is_unchanged_ascii() {
    let input = "echo hello world";
    let output = format_command_text(input);
    assert_eq!(output, input);
}

#[test]
fn single_line_without_newline_preserves_multibyte_characters() {
    let input = "echo 🚀✨";
    let output = format_command_text(input);
    assert_eq!(output, input);

    // Additional sanity check: string is valid UTF-8 and can be iterated by chars without panic
    let collected: String = output.chars().collect();
    assert_eq!(collected, output);
}

#[test]
fn truncates_at_first_newline_and_appends_ellipsis_when_more_content_exists() {
    let input = "cargo build\n--release";
    let output = format_command_text(input);
    assert_eq!(output, "cargo build…");
}

#[test]
fn truncates_at_first_newline_without_ellipsis_when_rest_is_whitespace() {
    let input = "git status\n   \t  ";
    let output = format_command_text(input);
    assert_eq!(output, "git status");
}

#[test]
fn does_not_split_multibyte_char_across_utf8_boundaries_when_newline_follows() {
    // The emoji is a multi-byte sequence; ensure truncation at the newline does not split it.
    let input = "echo 🧪\nthen do something";
    let output = format_command_text(input);
    assert_eq!(output, "echo 🧪…");

    // Validate resulting string is valid UTF-8 by iterating graphemes via chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn preserves_combining_characters_when_newline_is_after_cluster() {
    // "e" + combining acute accent
    // Sanity checks that the formatter doesn't split this unicode sequence
    let composed = format!("{}{}", 'e', '\u{0301}');
    let input = format!("echo {composed}\nnext");
    let output = format_command_text(&input);
    assert_eq!(output, format!("echo {composed}…"));

    // Still valid UTF-8 and same when re-collected from chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn newline_then_multibyte_results_in_ellipsis_only() {
    let input = "\n🚀";
    let output = format_command_text(input);
    assert_eq!(output, "…");

    // Sanity: output remains valid UTF-8
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn mcp_blocked_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_blocked_title_text("create_issue", Some("github")),
        "OK if I call MCP tool create_issue on server github"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_tool_name_when_server_unknown() {
    assert_eq!(
        mcp_blocked_title_text("create_issue", None),
        "OK if I call MCP tool create_issue"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_blocked_title_text("", Some("github")),
        "OK if I call this MCP tool?"
    );
    assert_eq!(
        mcp_blocked_title_text("", None),
        "OK if I call this MCP tool?"
    );
}

#[test]
fn mcp_viewing_detail_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", Some("github")),
        "Viewing MCP tool create_issue on github"
    );
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", None),
        "Viewing MCP tool create_issue"
    );
}

#[test]
fn mcp_viewing_detail_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_viewing_detail_title_text("", Some("github")),
        "Viewing MCP tool call detail"
    );
}

#[test]
fn command_highlight_color_ranges_maps_described_tokens_to_colors() {
    let parsed = ParsedTokensSnapshot {
        buffer_text: "echo hello".to_string(),
        parsed_tokens: vec![
            parsed_token(
                "echo",
                (0, 4),
                Some(SuggestionType::Command(
                    TopLevelCommandCaseSensitivity::CaseSensitive,
                )),
            ),
            // Undescribed tokens (e.g. unrecognized arguments) are left uncolored, matching the
            // terminal input's own behavior.
            parsed_token("hello", (5, 10), None),
        ],
    };

    let colors = test_terminal_colors();
    let ranges = command_highlight_color_ranges(&parsed, &colors);

    assert_eq!(
        ranges,
        vec![(
            CharOffset::from(0)..CharOffset::from(4),
            ColorU::from(colors.green)
        )]
    );
}

#[test]
fn command_highlight_color_ranges_converts_byte_spans_to_char_offsets() {
    // "echo " (5 bytes/chars) + "🚀" (4 bytes, 1 char) + " " (1 byte/char) + "arg" starting at
    // byte 10, char 7.
    let text = "echo 🚀 arg";
    let parsed = ParsedTokensSnapshot {
        buffer_text: text.to_string(),
        parsed_tokens: vec![parsed_token(
            "arg",
            (10, 13),
            Some(SuggestionType::Argument),
        )],
    };

    let colors = test_terminal_colors();
    let ranges = command_highlight_color_ranges(&parsed, &colors);

    assert_eq!(
        ranges,
        vec![(
            CharOffset::from(7)..CharOffset::from(10),
            ColorU::from(colors.cyan)
        )]
    );
}

#[test]
fn header_highlight_ranges_passes_through_when_untruncated() {
    let color_ranges = vec![(CharOffset::from(0)..CharOffset::from(4), ColorU::black())];
    let highlighted = header_highlight_ranges(&color_ranges, None);

    assert_eq!(highlighted.len(), 1);
    assert_eq!(highlighted[0].highlight_indices, vec![0, 1, 2, 3]);
}

#[test]
fn header_highlight_ranges_clips_range_straddling_the_truncation_boundary() {
    let color_ranges = vec![(CharOffset::from(2)..CharOffset::from(8), ColorU::black())];
    let highlighted = header_highlight_ranges(&color_ranges, Some(5));

    assert_eq!(highlighted.len(), 1);
    assert_eq!(highlighted[0].highlight_indices, vec![2, 3, 4]);
}

#[test]
fn header_highlight_ranges_drops_range_entirely_past_the_truncation_boundary() {
    let color_ranges = vec![(CharOffset::from(6)..CharOffset::from(9), ColorU::black())];
    let highlighted = header_highlight_ranges(&color_ranges, Some(5));

    assert!(highlighted.is_empty());
}

#[test]
fn syntax_highlighting_setting_toggle_updates_header_highlighting() {
    use std::rc::Rc;

    use settings::Setting as _;
    use warpui::elements::{Empty, MouseStateHandle};
    use warpui::platform::WindowStyle;
    use warpui::{
        App, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewHandle,
    };

    use super::{RequestedActionViewType, RequestedCommandView};
    use crate::ai::agent::conversation::AIConversationId;
    use crate::ai::agent::{AIAgentActionId, AIAgentExchangeId};
    use crate::ai::blocklist::block::AutonomySettingSpeedbump;
    use crate::ai::blocklist::model::AIBlockModel;
    use crate::ai::blocklist::{AIBlock, ClientIdentifiers, FakeAIBlockModel};
    use crate::settings::InputSettings;
    use crate::test_util::assert_eventually;
    use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

    /// Minimal host view so `RequestedCommandView` can be constructed via
    /// `ctx.add_typed_action_view` without pulling in the full `AIBlock` stack.
    struct Host {
        view: ViewHandle<RequestedCommandView>,
    }
    impl Entity for Host {
        type Event = ();
    }
    impl View for Host {
        fn ui_name() -> &'static str {
            "RequestedCommandViewTestHost"
        }
        fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
    }
    impl TypedActionView for Host {
        type Action = ();
    }

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(crate::notebooks::editor::keys::NotebookKeybindings::new);

        // Reuse a real `TerminalView`'s already-fully-wired `BlocklistAIActionModel` and
        // `TerminalModel` rather than reconstructing that dependency graph from scratch.
        let terminal = add_window_with_terminal(&mut app, None);
        let (action_model, terminal_model) = terminal.read(&app, |view, _| {
            (view.ai_action_model().clone(), view.model.clone())
        });

        // Start with syntax highlighting off, mirroring a card created while the setting is off.
        InputSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.syntax_highlighting.set_value(false, ctx);
        });

        let (_window_id, host) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            let view = ctx.add_typed_action_view(move |ctx| {
                let block_model: Rc<dyn AIBlockModel<View = AIBlock>> =
                    Rc::new(FakeAIBlockModel::new_streaming(vec![]));
                let mut view = RequestedCommandView::new(
                    AIAgentActionId::from("test-action".to_owned()),
                    ClientIdentifiers {
                        conversation_id: AIConversationId::new(),
                        client_exchange_id: AIAgentExchangeId::new(),
                        response_stream_id: None,
                    },
                    RequestedActionViewType::Command,
                    block_model,
                    &action_model,
                    terminal_model,
                    AutonomySettingSpeedbump::None,
                    MouseStateHandle::default(),
                    EntityId::new(),
                    ctx,
                );
                view.apply_streamed_update("git status", ctx);
                view.ensure_editor(ctx);
                view
            });
            Host { view }
        });
        let view = host.read(&app, |host, _| host.view.clone());

        assert_eventually!(
            view.read(&app, |view, ctx| view
                .command_highlighted_ranges_for_header(ctx)
                .is_empty()),
            "highlighting should stay off while the setting is disabled"
        );

        // Turning the setting on should highlight a card that was created while it was off.
        InputSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.syntax_highlighting.set_value(true, ctx);
        });

        assert_eventually!(
            view.read(&app, |view, ctx| !view
                .command_highlighted_ranges_for_header(ctx)
                .is_empty()),
            "turning syntax highlighting on should highlight an already-open card"
        );

        // Turning it back off should clear it again.
        InputSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.syntax_highlighting.set_value(false, ctx);
        });

        assert_eventually!(
            view.read(&app, |view, ctx| view
                .command_highlighted_ranges_for_header(ctx)
                .is_empty()),
            "turning syntax highlighting back off should clear highlighting"
        );
    });
}

#[test]
fn editing_the_permission_prompt_reparses_the_live_buffer() {
    use std::rc::Rc;

    use vec1::vec1;
    use warpui::elements::{Empty, MouseStateHandle};
    use warpui::platform::WindowStyle;
    use warpui::{App, Element, Entity, EntityId, TypedActionView, View, ViewHandle};

    use super::{RequestedActionViewType, RequestedCommandView};
    use crate::ai::agent::conversation::AIConversationId;
    use crate::ai::agent::{AIAgentActionId, AIAgentExchangeId};
    use crate::ai::blocklist::block::AutonomySettingSpeedbump;
    use crate::ai::blocklist::model::AIBlockModel;
    use crate::ai::blocklist::{AIBlock, ClientIdentifiers, FakeAIBlockModel};
    use crate::test_util::assert_eventually;
    use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

    /// Minimal host view so `RequestedCommandView` can be constructed via
    /// `ctx.add_typed_action_view` without pulling in the full `AIBlock` stack.
    struct Host {
        view: ViewHandle<RequestedCommandView>,
    }
    impl Entity for Host {
        type Event = ();
    }
    impl View for Host {
        fn ui_name() -> &'static str {
            "RequestedCommandViewEditModeTestHost"
        }
        fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
    }
    impl TypedActionView for Host {
        type Action = ();
    }

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(crate::notebooks::editor::keys::NotebookKeybindings::new);

        // Reuse a real `TerminalView`'s already-fully-wired `BlocklistAIActionModel` and
        // `TerminalModel` rather than reconstructing that dependency graph from scratch.
        let terminal = add_window_with_terminal(&mut app, None);
        let (action_model, terminal_model) = terminal.read(&app, |view, _| {
            (view.ai_action_model().clone(), view.model.clone())
        });

        let (_window_id, host) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            let view = ctx.add_typed_action_view(move |ctx| {
                let block_model: Rc<dyn AIBlockModel<View = AIBlock>> =
                    Rc::new(FakeAIBlockModel::new_streaming(vec![]));
                let mut view = RequestedCommandView::new(
                    AIAgentActionId::from("test-action".to_owned()),
                    ClientIdentifiers {
                        conversation_id: AIConversationId::new(),
                        client_exchange_id: AIAgentExchangeId::new(),
                        response_stream_id: None,
                    },
                    RequestedActionViewType::Command,
                    block_model,
                    &action_model,
                    terminal_model,
                    AutonomySettingSpeedbump::None,
                    MouseStateHandle::default(),
                    EntityId::new(),
                    ctx,
                );
                view.apply_streamed_update("git status", ctx);
                view.ensure_editor(ctx);
                view
            });
            Host { view }
        });
        let view = host.read(&app, |host, _| host.view.clone());

        // Wait for the initial background parse of "git status" to complete and push colors
        // into the editor.
        assert_eventually!(
            view.read(&app, |view, ctx| {
                view.editor.as_ref().is_some_and(|editor| {
                    !editor
                        .as_ref(ctx)
                        .model
                        .as_ref(ctx)
                        .external_highlight_colors_for_test()
                        .is_empty()
                })
            }),
            "expected initial highlight colors on the editor"
        );

        // Enter edit mode (mirrors clicking "Edit" on the permission prompt) and simulate a user
        // edit appending " && pwd" past the original 10-character "git status" text.
        view.update(&mut app, |view, ctx| view.open_edit_mode(ctx));
        let editor = view.read(&app, |view, _| view.editor.clone().expect("editor exists"));
        editor.update(&mut app, |editor, ctx| {
            let end = editor.model.as_ref(ctx).max_character_offset(ctx);
            editor.apply_edits(vec1![(" && pwd".to_string(), end..end)], ctx);
        });

        // The reparse triggered by the user edit should eventually produce a highlight range past
        // the original text's length, proving it reflects the live edited buffer (e.g. `pwd`
        // getting colored as a command) rather than staying stale at the pre-edit offsets.
        assert_eventually!(
            editor.read(&app, |editor, ctx| {
                editor
                    .model
                    .as_ref(ctx)
                    .external_highlight_colors_for_test()
                    .iter()
                    .any(|(range, _)| range.end.as_usize() > 10)
            }),
            "editing the permission prompt should reparse the live buffer, not just the original \
             command_text"
        );
    });
}
