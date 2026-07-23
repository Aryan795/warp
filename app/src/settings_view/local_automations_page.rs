//! Settings page hosting the Local Automations list.

use warpui::elements::{ChildView, Element};
use warpui::{AppContext, Entity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
};
use super::SettingsSection;
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::local_automations::list_view::LocalAutomationsViewEvent;
use crate::local_automations::LocalAutomationsView;

const PAGE_TITLE_TEXT: &str = "Automations";

pub struct LocalAutomationsSettingsPageView {
    page: PageType<Self>,
    list_view: ViewHandle<LocalAutomationsView>,
}

impl LocalAutomationsSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let list_view = ctx.add_typed_action_view(LocalAutomationsView::new);
        // Bubble modal open/close up to the hosting settings view so it
        // re-renders the modal overlay from `get_modal_content`.
        ctx.subscribe_to_view(&list_view, |_, _, event, ctx| {
            ctx.emit(*event);
            ctx.notify();
        });
        Self {
            page: PageType::new_monolith(
                LocalAutomationsSettingsWidget,
                Some(PAGE_TITLE_TEXT),
                true,
            ),
            list_view,
        }
    }

    /// The Automations agent modal overlay when it is open, rendered by the
    /// hosting settings view above the whole settings surface.
    pub fn get_modal_content(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        self.list_view.as_ref(app).agent_modal_content(app)
    }
}

impl Entity for LocalAutomationsSettingsPageView {
    type Event = LocalAutomationsViewEvent;
}

impl View for LocalAutomationsSettingsPageView {
    fn ui_name() -> &'static str {
        "LocalAutomationsSettingsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for LocalAutomationsSettingsPageView {
    type Action = ();
}

impl SettingsPageMeta for LocalAutomationsSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::LocalAutomations
    }

    fn on_page_selected(&mut self, allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        self.list_view.update(ctx, |view, ctx| {
            view.on_open(ctx);
        });
        if allow_steal_focus {
            ctx.focus(&self.list_view);
        }
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        cfg!(not(target_family = "wasm")) && FeatureFlag::LocalAutomations.is_enabled()
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget()
    }
}

impl From<ViewHandle<LocalAutomationsSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<LocalAutomationsSettingsPageView>) -> Self {
        SettingsPageViewHandle::LocalAutomations(view_handle)
    }
}

struct LocalAutomationsSettingsWidget;

impl SettingsWidget for LocalAutomationsSettingsWidget {
    type View = LocalAutomationsSettingsPageView;

    fn search_terms(&self) -> &str {
        "automations local automation schedule cron run now open config"
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        ChildView::new(&view.list_view).finish()
    }
}
