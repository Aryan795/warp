use std::net::TcpListener;
use std::time::Duration;

use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use ai::grok_subscription::oauth::callback_addr;
// `std::time::Instant` is disallowed (no wasm support); `instant::Instant` is a
// drop-in that re-exports the std type on native targets.
use instant::Instant;
use warp::editor::CodeEditorModel;
use warp::settings::AISettings;
use warp::tui_export::register_tui_session_view_test_singletons;
use warp_core::features::FeatureFlag;
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::{App, ModelHandle};

use super::{TuiApiKeysFooter, TuiApiKeysMenuModel, input_text};
use crate::inline_menu::TuiInlineMenuInputOwnership;
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

fn add_menu(
    app: &mut App,
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiInputSuggestionsModeModel>,
    ModelHandle<TuiApiKeysMenuModel>,
) {
    register_tui_session_view_test_singletons(app);
    app.update(|ctx| {
        let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
        let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
        let menu = ctx.add_model(|ctx| TuiApiKeysMenuModel::new(input.clone(), mode.clone(), ctx));
        menu.update(ctx, |menu, ctx| menu.open(ctx));
        (input, mode, menu)
    })
}

#[test]
fn changing_the_shared_menu_mode_deactivates_api_keys_state() {
    App::test((), |mut app| async move {
        let (input, mode, menu) = add_menu(&mut app);
        input.update(&mut app, |input, ctx| input.user_insert("query", ctx));
        mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::ModelSelector, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ModelSelector
            );
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::Composer
            );
            assert!(!menu.as_ref(ctx).uses_credential_border(ctx));
            assert_eq!(menu.as_ref(ctx).footer(ctx), None);
            assert_eq!(input_text(&input, ctx), "");
        });
    });
}

#[test]
fn browsing_rows_are_alphabetical_with_fallback_last() {
    App::test((), |mut app| async move {
        let (_, mode, menu) = add_menu(&mut app);
        app.read(|ctx| {
            assert_eq!(mode.as_ref(ctx).mode(), TuiInputSuggestionsMode::ApiKeys);
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            );
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .map(|row| row.title.as_str())
                    .collect::<Vec<_>>(),
                vec![
                    "Anthropic API key",
                    "Google API key",
                    "OpenAI API key",
                    "X premium or SuperGrok subscription",
                    "Warp credit fallback",
                ]
            );
            assert_eq!(snapshot.selected_index, Some(0));
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { can_clear: false })
            );
        });
    });
}

#[test]
fn filtering_keeps_warp_credit_fallback_pinned() {
    App::test((), |mut app| async move {
        let (input, _, menu) = add_menu(&mut app);
        input.update(&mut app, |input, ctx| input.user_insert("google", ctx));
        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .map(|row| row.title.as_str())
                    .collect::<Vec<_>>(),
                vec!["Google API key", "Warp credit fallback"]
            );
        });
    });
}

#[test]
fn connected_provider_prefills_secret_input_and_saves_replacement() {
    App::test((), |mut app| async move {
        let (input, _, menu) = add_menu(&mut app);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(
                    LLMProvider::Anthropic,
                    Some("existing-secret".to_owned()),
                    ctx,
                )
            })
            .unwrap();

        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuMasked
            );
            assert_eq!(input_text(&input, ctx), "existing-secret");
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::EditingProvider(LLMProvider::Anthropic))
            );
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer(ctx);
            input.user_insert("replacement-secret", ctx);
        });
        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert_eq!(
                ApiKeyManager::as_ref(ctx).keys().anthropic.as_deref(),
                Some("replacement-secret")
            );
            assert_eq!(input_text(&input, ctx), "");
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            );
        });
    });
}

#[test]
fn open_and_connect_grok_matches_selecting_the_grok_row() {
    App::test((), |mut app| async move {
        // Without these the Grok row bounces off its policy gates and both
        // paths agree on an error state, which would compare equal for the
        // wrong reason.
        let _super_grok = FeatureFlag::SuperGrok.override_enabled(true);
        let _byok = FeatureFlag::SoloUserByok.override_enabled(true);
        register_tui_session_view_test_singletons(&mut app);

        // Reference path: open the menu, then select and accept the Grok row.
        let reference = app.update(|ctx| {
            let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let menu = ctx.add_model(|ctx| TuiApiKeysMenuModel::new(input, mode, ctx));
            menu.update(ctx, |menu, ctx| {
                menu.open(ctx);
                assert!(menu.select_at_snapshot_index(3, ctx));
                menu.accept_selected(ctx);
            });
            menu
        });
        let (expected_footer, expected_snapshot) = app.read(|ctx| {
            (
                reference.as_ref(ctx).footer(ctx),
                reference.as_ref(ctx).snapshot(ctx),
            )
        });
        assert_eq!(
            expected_footer,
            Some(TuiApiKeysFooter::ConnectingGrok),
            "selecting the Grok row should start connecting",
        );

        // Only one attempt can hold the loopback callback port, so the
        // reference attempt has to be torn down before the shortcut starts its
        // own — otherwise the shortcut fails to bind and falls back to an
        // error state that has nothing to do with the two paths differing.
        reference.update(&mut app, |menu, ctx| menu.dismiss(ctx));
        wait_for_grok_callback_port();

        // Shortcut path: a single call jumps straight into the Grok connect flow.
        let shortcut = app.update(|ctx| {
            let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let menu = ctx.add_model(|ctx| TuiApiKeysMenuModel::new(input, mode, ctx));
            menu.update(ctx, |menu, ctx| menu.open_and_connect_grok(ctx));
            menu
        });

        app.read(|ctx| {
            assert!(shortcut.as_ref(ctx).is_open(ctx));
            assert_eq!(
                shortcut.as_ref(ctx).footer(ctx),
                expected_footer,
                "the shortcut should land in the same footer state as selecting the Grok row",
            );
            // The whole snapshot, not just its header: the header reads
            // "API keys" in the browsing state too, so comparing it alone
            // would still hold if the shortcut never started connecting.
            assert_eq!(shortcut.as_ref(ctx).snapshot(ctx), expected_snapshot);
        });
    });
}

/// Blocks until a cancelled attempt has released the Grok callback listener.
/// The loopback server owns it on a dedicated thread that notices cancellation
/// only on its poll interval, so the port frees shortly after the attempt is
/// cancelled rather than synchronously.
fn wait_for_grok_callback_port() {
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Err(error) = TcpListener::bind(callback_addr()) {
        assert!(
            Instant::now() < deadline,
            "the Grok callback port was never released: {error}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn connecting_grok_invites_the_user_to_paste_the_sign_in_code() {
    App::test((), |mut app| async move {
        let _super_grok = FeatureFlag::SuperGrok.override_enabled(true);
        let _byok = FeatureFlag::SoloUserByok.override_enabled(true);
        let (_, _, menu) = add_menu(&mut app);
        app.read(|ctx| {
            assert_eq!(menu.as_ref(ctx).input_placeholder_ghost_text(ctx), None);
        });

        // Editing a plain provider key has no manual code to paste.
        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::EditingProvider(LLMProvider::Anthropic))
            );
            assert_eq!(menu.as_ref(ctx).input_placeholder_ghost_text(ctx), None);
        });

        menu.update(&mut app, |menu, ctx| {
            menu.dismiss(ctx);
            menu.open_and_connect_grok(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::ConnectingGrok),
                "the Grok row must reach the connecting state"
            );
            assert_eq!(
                menu.as_ref(ctx).input_placeholder_ghost_text(ctx),
                Some("Paste sign-in code")
            );
        });

        // Backing out of the connect flow drops the invitation.
        menu.update(&mut app, |menu, ctx| menu.dismiss(ctx));
        app.read(|ctx| {
            assert_eq!(menu.as_ref(ctx).input_placeholder_ghost_text(ctx), None);
        });
    });
}

#[test]
fn clear_selected_provider_and_toggle_fallback_keep_menu_open() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("secret".to_owned()), ctx)
            })
            .unwrap();
        menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(2, ctx));
            assert_eq!(
                menu.footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { can_clear: true })
            );
            menu.clear_selected(ctx);
        });
        app.read(|ctx| {
            assert_eq!(ApiKeyManager::as_ref(ctx).keys().openai, None);
            assert!(menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                menu.as_ref(ctx).snapshot(ctx).unwrap().selected_index,
                Some(2)
            );
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { can_clear: false })
            );
        });

        menu.update(&mut app, |menu, ctx| {
            menu.select_at_snapshot_index(4, ctx);
            menu.accept_selected(ctx);
        });
        app.read(|ctx| {
            assert!(*AISettings::as_ref(ctx).can_use_warp_credits_for_fallback);
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::WarpCreditFallback)
            );
            assert!(menu.as_ref(ctx).is_open(ctx));
        });
    });
}
