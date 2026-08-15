use std::sync::Arc;

use string_offset::ByteOffset;
use warp_completer::completer;
use warp_completer::completer::Description;
use warpui::accessibility::{AccessibilityContent, WarpA11yRole};
use warpui::{AppContext, View, ViewContext};

use crate::completer::{SessionAgnosticContext, SessionContext};
use crate::server::telemetry::CommandXRayTrigger;

/// The completion context a host describes tokens against.
///
/// [`completer::describe`] is generic over [`warp_completer::completer::CompletionContext`], so
/// hosts hand back one of the concrete contexts rather than a trait object.
#[derive(Clone)]
pub enum CommandXRayContext {
    /// A live shell session: aliases, functions, `$PATH`, and path completions all resolve.
    Session(SessionContext),
    /// No session available; only the static command registry can be consulted.
    SessionAgnostic(SessionAgnosticContext),
}

impl CommandXRayContext {
    async fn describe(&self, line: &str, pos: ByteOffset) -> Option<Description> {
        match self {
            CommandXRayContext::Session(context) => completer::describe(line, pos, context).await,
            CommandXRayContext::SessionAgnostic(context) => {
                completer::describe(line, pos, context).await
            }
        }
    }
}

/// How a host should apply a change to the x-ray it is showing.
pub enum CommandXRayUpdate {
    /// Show this description. The host stores it and mirrors it onto its text surface, which
    /// needs it to hit-test the token bounds and to anchor the tooltip.
    Show(Arc<Description>),
    /// The describe produced nothing for the probed offset. The host drops any stored
    /// description without dismissing the hover, so a subsequent hover on the same spot can
    /// still open.
    Empty,
    /// Hide the tooltip and dismiss the hover, so it stays closed until the pointer moves.
    Dismiss,
}

/// The seam a view implements to get command x-ray.
///
/// Everything above this trait — the hover state machine, the describe call, the tooltip — is
/// shared. Everything below it is per-host: where the command text comes from, which completion
/// context describes it, and how the description reaches the host's text surface. Pointer
/// geometry is deliberately *not* part of this trait: hit testing happens in each host's element,
/// against layout only that element has.
pub trait CommandXRayHost: View + Sized {
    /// The command text that x-ray byte offsets index into.
    fn x_ray_command_text(&self, ctx: &AppContext) -> String;

    /// The completion context to describe against. `None` disables x-ray for this host.
    fn x_ray_context(&self, ctx: &AppContext) -> Option<CommandXRayContext>;

    /// The description currently being shown, if any.
    fn x_ray_description(&self) -> Option<&Arc<Description>>;

    /// Applies an x-ray update to the host and its text surface.
    fn apply_x_ray_update(&mut self, update: CommandXRayUpdate, ctx: &mut ViewContext<Self>);
}

/// Hides the x-ray, if one is showing, and dismisses the hover.
pub fn hide<V: CommandXRayHost>(view: &mut V, ctx: &mut ViewContext<V>) {
    if view.x_ray_description().is_some() {
        view.apply_x_ray_update(CommandXRayUpdate::Dismiss, ctx);
        ctx.notify();
    }
}

/// Describes the token at `pos` and shows it, if there is anything to describe.
pub fn start_at_offset<V: CommandXRayHost>(
    view: &mut V,
    pos: ByteOffset,
    trigger: CommandXRayTrigger,
    ctx: &mut ViewContext<V>,
) {
    let Some(context) = view.x_ray_context(ctx) else {
        return;
    };
    let command_text = view.x_ray_command_text(ctx);
    let _ = ctx.spawn(
        async move { context.describe(command_text.as_str(), pos).await },
        move |view, description, ctx| {
            show(view, description, trigger, ctx);
        },
    );
}

/// Toggles the x-ray at `pos`: hides it if one is already showing, otherwise describes and shows.
/// This is the keyboard path.
pub fn toggle_at_offset<V: CommandXRayHost>(
    view: &mut V,
    pos: ByteOffset,
    ctx: &mut ViewContext<V>,
) {
    if view.x_ray_description().is_some() {
        hide(view, ctx);
    } else {
        start_at_offset(view, pos, CommandXRayTrigger::Keystroke, ctx);
    }
}

fn show<V: CommandXRayHost>(
    view: &mut V,
    description: Option<Description>,
    trigger: CommandXRayTrigger,
    ctx: &mut ViewContext<V>,
) {
    match description.map(Arc::new) {
        Some(description) => {
            if trigger == CommandXRayTrigger::Keystroke {
                ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                    description.a11y_text(),
                    WarpA11yRole::UserAction,
                ));
            }
            ctx.notify();
            view.apply_x_ray_update(CommandXRayUpdate::Show(description), ctx);
        }
        None => view.apply_x_ray_update(CommandXRayUpdate::Empty, ctx),
    }
    ctx.notify();
}
