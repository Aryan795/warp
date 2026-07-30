use warpui::elements::Empty;
use warpui::{App, AppContext, Element, Entity, View};

use super::{LOCAL_CONTROL_MODE_SEARCH_TERMS, WARP_CONTROL_CLI_INSTALL_SEARCH_TERMS};
use crate::appearance::Appearance;
use crate::settings_view::settings_page::{FilteredPageType, MatchData, PageType, SettingsWidget};

struct TestScriptingView;

impl Entity for TestScriptingView {
    type Event = ();
}

impl View for TestScriptingView {
    fn ui_name() -> &'static str {
        "TestScriptingView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

struct StubWidget(&'static str);

impl SettingsWidget for StubWidget {
    type View = TestScriptingView;

    fn search_terms(&self) -> &str {
        self.0
    }

    fn render(&self, _: &Self::View, _: &Appearance, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

fn scripting_page() -> PageType<TestScriptingView> {
    PageType::new_uncategorized(
        vec![
            Box::new(StubWidget(WARP_CONTROL_CLI_INSTALL_SEARCH_TERMS)),
            Box::new(StubWidget(LOCAL_CONTROL_MODE_SEARCH_TERMS)),
        ],
        Some("Scripting"),
    )
}

fn visible_search_terms(page: &PageType<TestScriptingView>) -> Vec<&str> {
    let FilteredPageType::Uncategorized { widgets, .. } = page.get_filtered() else {
        panic!("expected Uncategorized Scripting page");
    };
    widgets
        .into_iter()
        .map(SettingsWidget::search_terms)
        .collect()
}

#[test]
fn scripting_search_filters_to_the_matching_control() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let mut page = scripting_page();

            let install_matches = page.update_filter("install", ctx);
            assert!(matches!(install_matches, MatchData::Countable(1)));
            assert_eq!(
                visible_search_terms(&page),
                vec![WARP_CONTROL_CLI_INSTALL_SEARCH_TERMS]
            );

            let scripting_matches = page.update_filter("scripting", ctx);
            assert!(matches!(scripting_matches, MatchData::Countable(1)));
            assert_eq!(
                visible_search_terms(&page),
                vec![LOCAL_CONTROL_MODE_SEARCH_TERMS]
            );

            let automation_matches = page.update_filter("automation", ctx);
            assert!(matches!(automation_matches, MatchData::Countable(1)));
            assert_eq!(
                visible_search_terms(&page),
                vec![LOCAL_CONTROL_MODE_SEARCH_TERMS]
            );
        });
    });
}

#[test]
fn clearing_scripting_search_restores_all_controls() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let mut page = scripting_page();
            page.update_filter("install", ctx);
            assert_eq!(
                visible_search_terms(&page),
                vec![WARP_CONTROL_CLI_INSTALL_SEARCH_TERMS]
            );

            let empty_matches = page.update_filter("", ctx);
            assert!(matches!(empty_matches, MatchData::Uncounted(true)));
            assert_eq!(
                visible_search_terms(&page),
                vec![
                    WARP_CONTROL_CLI_INSTALL_SEARCH_TERMS,
                    LOCAL_CONTROL_MODE_SEARCH_TERMS
                ]
            );
        });
    });
}
