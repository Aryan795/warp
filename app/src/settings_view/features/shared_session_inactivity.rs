use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use settings::Setting;
use warp_errors::report_if_error;
use warpui::elements::{
    Container, CrossAxisAlignment, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{self, EditorView, SingleLineEditorOptions, TextOptions};
use crate::settings_view::settings_page::{LocalOnlyIconState, ToggleState, render_body_item};
use crate::terminal::shared_session::settings::{
    InactivityPeriodBeforeEndingSession, InactivityPeriodBeforeRevokingRoles,
    InactivityPeriodBeforeWarning, SharedSessionSettings, SharedSessionSettingsChangedEvent,
};

/// Minimum allowed value for any of the inactivity durations, in minutes.
const MIN_MINUTES: u64 = 1;

#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
pub enum Action {
    /// The "revoke edit access after" duration editor was committed.
    RevokeEditAccessAfterChanged,
    /// The "warn before ending session after" duration editor was committed.
    WarningAfterChanged,
    /// The "end session after" duration editor was committed.
    EndSessionAfterChanged,
}

/// A view containing settings that control how long a shared session can sit
/// idle before the sharer's edit access ladder kicks in: edit access is
/// revoked, then a warning is shown, then the session ends. This view only
/// exposes those existing durations for editing; it does not change the
/// underlying inactivity behavior.
pub struct SharedSessionInactivityView {
    revoke_edit_access_editor: ViewHandle<EditorView>,
    warning_editor: ViewHandle<EditorView>,
    end_session_editor: ViewHandle<EditorView>,
    is_revoke_edit_access_valid: bool,
    is_warning_valid: bool,
    is_end_session_valid: bool,
    local_only_icon_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl SharedSessionInactivityView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editor_options = SingleLineEditorOptions {
            text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
            ..Default::default()
        };

        let revoke_edit_access_editor =
            ctx.add_typed_action_view(|ctx| EditorView::single_line(editor_options.clone(), ctx));
        let warning_editor =
            ctx.add_typed_action_view(|ctx| EditorView::single_line(editor_options.clone(), ctx));
        let end_session_editor =
            ctx.add_typed_action_view(|ctx| EditorView::single_line(editor_options.clone(), ctx));

        ctx.subscribe_to_model(
            &SharedSessionSettings::handle(ctx),
            |me, settings, event, ctx| {
                match event {
                    SharedSessionSettingsChangedEvent::InactivityPeriodBeforeRevokingRoles {
                        ..
                    } => {
                        me.revoke_edit_access_editor.update(ctx, |editor, ctx| {
                            let minutes = Self::minutes(
                                *settings.as_ref(ctx).inactivity_period_before_revoking_roles,
                            );
                            editor.set_buffer_text(&minutes.to_string(), ctx);
                        });
                    }
                    SharedSessionSettingsChangedEvent::InactivityPeriodBeforeWarning { .. } => {
                        me.warning_editor.update(ctx, |editor, ctx| {
                            let minutes = Self::minutes(
                                *settings.as_ref(ctx).inactivity_period_before_warning,
                            );
                            editor.set_buffer_text(&minutes.to_string(), ctx);
                        });
                    }
                    SharedSessionSettingsChangedEvent::InactivityPeriodBeforeEndingSession {
                        ..
                    } => {
                        me.end_session_editor.update(ctx, |editor, ctx| {
                            let minutes = Self::minutes(
                                *settings.as_ref(ctx).inactivity_period_before_ending_session,
                            );
                            editor.set_buffer_text(&minutes.to_string(), ctx);
                        });
                    }
                    _ => {}
                }
                ctx.notify();
            },
        );

        ctx.subscribe_to_view(&revoke_edit_access_editor, move |me, _, event, ctx| {
            me.handle_revoke_edit_access_editor_event(event, ctx);
        });
        ctx.subscribe_to_view(&warning_editor, move |me, _, event, ctx| {
            me.handle_warning_editor_event(event, ctx);
        });
        ctx.subscribe_to_view(&end_session_editor, move |me, _, event, ctx| {
            me.handle_end_session_editor_event(event, ctx);
        });

        let settings = SharedSessionSettings::as_ref(ctx);
        let revoke_minutes = Self::minutes(*settings.inactivity_period_before_revoking_roles);
        let warning_minutes = Self::minutes(*settings.inactivity_period_before_warning);
        let end_minutes = Self::minutes(*settings.inactivity_period_before_ending_session);

        revoke_edit_access_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&revoke_minutes.to_string(), ctx);
        });
        warning_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&warning_minutes.to_string(), ctx);
        });
        end_session_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&end_minutes.to_string(), ctx);
        });

        Self {
            revoke_edit_access_editor,
            warning_editor,
            end_session_editor,
            is_revoke_edit_access_valid: true,
            is_warning_valid: true,
            is_end_session_valid: true,
            local_only_icon_states: Default::default(),
        }
    }

    /// Parses user-entered text into a positive number of minutes, returning
    /// `None` if the text isn't a valid, positive integer.
    fn parse_minutes(text: &str) -> Option<u64> {
        text.trim()
            .parse::<u64>()
            .ok()
            .filter(|&minutes| minutes >= MIN_MINUTES)
    }

    /// Converts a duration to whole minutes for display, rounding up so that
    /// a duration is never displayed as zero minutes.
    fn minutes(duration: Duration) -> u64 {
        duration.as_secs().div_ceil(60).max(MIN_MINUTES)
    }

    fn handle_revoke_edit_access_editor_event(
        &mut self,
        event: &editor::Event,
        ctx: &mut ViewContext<Self>,
    ) {
        use editor::Event;
        match event {
            Event::Edited(_) => {
                let text = self.revoke_edit_access_editor.as_ref(ctx).buffer_text(ctx);
                let is_valid = Self::parse_minutes(&text).is_some();
                if is_valid != self.is_revoke_edit_access_valid {
                    self.is_revoke_edit_access_valid = is_valid;
                    ctx.notify();
                }
            }
            Event::Blurred | Event::Enter => {
                self.handle_action(&Action::RevokeEditAccessAfterChanged, ctx);
            }
            _ => (),
        }
    }

    fn handle_warning_editor_event(&mut self, event: &editor::Event, ctx: &mut ViewContext<Self>) {
        use editor::Event;
        match event {
            Event::Edited(_) => {
                let text = self.warning_editor.as_ref(ctx).buffer_text(ctx);
                let is_valid = Self::parse_minutes(&text).is_some();
                if is_valid != self.is_warning_valid {
                    self.is_warning_valid = is_valid;
                    ctx.notify();
                }
            }
            Event::Blurred | Event::Enter => {
                self.handle_action(&Action::WarningAfterChanged, ctx);
            }
            _ => (),
        }
    }

    fn handle_end_session_editor_event(
        &mut self,
        event: &editor::Event,
        ctx: &mut ViewContext<Self>,
    ) {
        use editor::Event;
        match event {
            Event::Edited(_) => {
                let text = self.end_session_editor.as_ref(ctx).buffer_text(ctx);
                let is_valid = Self::parse_minutes(&text).is_some();
                if is_valid != self.is_end_session_valid {
                    self.is_end_session_valid = is_valid;
                    ctx.notify();
                }
            }
            Event::Blurred | Event::Enter => {
                self.handle_action(&Action::EndSessionAfterChanged, ctx);
            }
            _ => (),
        }
    }

    /// Commits a new value for the "revoke edit access" duration, clamping it
    /// so the revoke -> warn -> end ordering is preserved. The other two
    /// durations are never modified as a result of this edit.
    fn commit_revoke_edit_access(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.revoke_edit_access_editor.as_ref(ctx).buffer_text(ctx);
        let Some(minutes) = Self::parse_minutes(&text) else {
            self.is_revoke_edit_access_valid = false;
            ctx.notify();
            return;
        };
        self.is_revoke_edit_access_valid = true;

        let settings = SharedSessionSettings::as_ref(ctx);
        let warning_minutes = Self::minutes(*settings.inactivity_period_before_warning);
        let end_minutes = Self::minutes(*settings.inactivity_period_before_ending_session);
        let clamped = minutes.min(warning_minutes).min(end_minutes);

        if clamped != minutes {
            self.revoke_edit_access_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&clamped.to_string(), ctx);
            });
        }

        let new_duration = Duration::from_secs(clamped * 60);
        if *SharedSessionSettings::as_ref(ctx).inactivity_period_before_revoking_roles
            != new_duration
        {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .inactivity_period_before_revoking_roles
                        .set_value(new_duration, ctx)
                );
            });
        }
        ctx.notify();
    }

    /// Commits a new value for the "warn" duration, clamping it so the
    /// revoke -> warn -> end ordering is preserved.
    fn commit_warning(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.warning_editor.as_ref(ctx).buffer_text(ctx);
        let Some(minutes) = Self::parse_minutes(&text) else {
            self.is_warning_valid = false;
            ctx.notify();
            return;
        };
        self.is_warning_valid = true;

        let settings = SharedSessionSettings::as_ref(ctx);
        let revoke_minutes = Self::minutes(*settings.inactivity_period_before_revoking_roles);
        let end_minutes = Self::minutes(*settings.inactivity_period_before_ending_session);
        let clamped = minutes.max(revoke_minutes).min(end_minutes);

        if clamped != minutes {
            self.warning_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&clamped.to_string(), ctx);
            });
        }

        let new_duration = Duration::from_secs(clamped * 60);
        if *SharedSessionSettings::as_ref(ctx).inactivity_period_before_warning != new_duration {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .inactivity_period_before_warning
                        .set_value(new_duration, ctx)
                );
            });
        }
        ctx.notify();
    }

    /// Commits a new value for the "end session" duration, clamping it so the
    /// revoke -> warn -> end ordering is preserved.
    fn commit_end_session(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.end_session_editor.as_ref(ctx).buffer_text(ctx);
        let Some(minutes) = Self::parse_minutes(&text) else {
            self.is_end_session_valid = false;
            ctx.notify();
            return;
        };
        self.is_end_session_valid = true;

        let settings = SharedSessionSettings::as_ref(ctx);
        let revoke_minutes = Self::minutes(*settings.inactivity_period_before_revoking_roles);
        let warning_minutes = Self::minutes(*settings.inactivity_period_before_warning);
        let clamped = minutes.max(revoke_minutes).max(warning_minutes);

        if clamped != minutes {
            self.end_session_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&clamped.to_string(), ctx);
            });
        }

        let new_duration = Duration::from_secs(clamped * 60);
        if *SharedSessionSettings::as_ref(ctx).inactivity_period_before_ending_session
            != new_duration
        {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(
                    settings
                        .inactivity_period_before_ending_session
                        .set_value(new_duration, ctx)
                );
            });
        }
        ctx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn render_duration_row(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        label: &str,
        description: &str,
        editor: &ViewHandle<EditorView>,
        is_valid: bool,
        storage_key: &str,
        sync_to_cloud: settings::SyncToCloud,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let border_color = if is_valid {
            None
        } else {
            Some(crate::themes::theme::Fill::error().into())
        };

        let editor_style = UiComponentStyles {
            width: Some(48.),
            padding: Some(Coords::uniform(5.)),
            background: Some(theme.surface_2().into()),
            border_color,
            ..Default::default()
        };

        let control = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                appearance
                    .ui_builder()
                    .text_input(editor.clone())
                    .with_style(editor_style)
                    .build()
                    .finish(),
            )
            .with_child(
                Container::new(
                    Text::new_inline(
                        "minutes",
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            )
            .finish();

        render_body_item::<Action>(
            label.to_string(),
            None,
            LocalOnlyIconState::for_setting(
                storage_key,
                sync_to_cloud,
                &mut self.local_only_icon_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            control,
            Some(description.to_string()),
        )
    }
}

impl Entity for SharedSessionInactivityView {
    type Event = ();
}

impl View for SharedSessionInactivityView {
    fn ui_name() -> &'static str {
        "SharedSessionInactivityView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_duration_row(
                appearance,
                app,
                "Revoke edit access after being inactive for",
                "Switches everyone you're sharing this session with to read-only after this much inactivity.",
                &self.revoke_edit_access_editor,
                self.is_revoke_edit_access_valid,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                InactivityPeriodBeforeRevokingRoles::sync_to_cloud(),
            ))
            .with_child(self.render_duration_row(
                appearance,
                app,
                "Warn before ending the session after being inactive for",
                "Shows a warning that the shared session is about to end.",
                &self.warning_editor,
                self.is_warning_valid,
                InactivityPeriodBeforeWarning::storage_key(),
                InactivityPeriodBeforeWarning::sync_to_cloud(),
            ))
            .with_child(self.render_duration_row(
                appearance,
                app,
                "End the shared session after being inactive for",
                "Automatically ends the shared session and disconnects everyone.",
                &self.end_session_editor,
                self.is_end_session_valid,
                InactivityPeriodBeforeEndingSession::storage_key(),
                InactivityPeriodBeforeEndingSession::sync_to_cloud(),
            ))
            .finish()
    }
}

impl TypedActionView for SharedSessionInactivityView {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::RevokeEditAccessAfterChanged => self.commit_revoke_edit_access(ctx),
            Action::WarningAfterChanged => self.commit_warning(ctx),
            Action::EndSessionAfterChanged => self.commit_end_session(ctx),
        }
    }
}
