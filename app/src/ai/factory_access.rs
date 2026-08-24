//! Tracks whether the signed-in viewer has Factory access, fetched once per authenticated
//! session so cloud-run links can route to Platform for enrolled viewers while the Factory
//! waitlist exists. See `specs/APP-5583/PRODUCT.md` and `specs/APP-5583/TECH.md`.

use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::server::server_api::ServerApiProvider;

/// The viewer's Factory access, as last resolved for the current authenticated session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FactoryAccess {
    /// Not yet resolved, or the resolution attempt timed out, failed, or returned a malformed
    /// response. Cloud-run links stay on Oz for the rest of the session; the check is not
    /// retried.
    #[default]
    Unknown,
    Allowed,
    Denied,
}

/// Emitted when the startup access check produces a result. Cloud-run links resolve again at
/// click time regardless, so subscribers only need this to repaint anything that displays the
/// access state itself.
pub enum FactoryAccessModelEvent {
    Resolved,
}

/// Application-scoped singleton holding the eager, once-per-session Factory access check.
///
/// One request fires after the first `AuthManagerEvent::AuthComplete` (or immediately at
/// construction if a persisted session is already logged in) and its result is held for the
/// rest of that authenticated session: no refresh timer, no retry, no foreground re-fetch.
/// `reset` is called from `auth::log_out` so the next authenticated session starts a fresh
/// check.
pub struct FactoryAccessModel {
    access: FactoryAccess,
    requested: bool,
    /// The in-flight probe, if any. Aborted on [`Self::reset`] so a response for a prior
    /// session cannot land after logout and apply to the next authenticated session.
    probe: Option<SpawnedFutureHandle>,
}

impl FactoryAccessModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| {
            if matches!(event, AuthManagerEvent::AuthComplete) {
                me.request_if_needed(ctx);
            }
        });

        let mut me = Self {
            access: FactoryAccess::Unknown,
            requested: false,
            probe: None,
        };
        if AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            me.request_if_needed(ctx);
        }
        me
    }

    pub fn access(&self) -> FactoryAccess {
        self.access
    }

    fn request_if_needed(&mut self, ctx: &mut ModelContext<Self>) {
        if self.requested {
            return;
        }
        self.requested = true;

        let client = ServerApiProvider::as_ref(ctx).get_factory_client();
        self.probe = Some(ctx.spawn(
            async move { client.get_factory_access().await },
            |me, result, ctx| {
                me.probe = None;
                me.access = match result {
                    Ok(response) if response.allowed => FactoryAccess::Allowed,
                    Ok(_) => FactoryAccess::Denied,
                    Err(error) => {
                        log::info!(
                            "Failed to resolve Factory access; cloud-run links stay on Oz \
                             for this session: {error:#}"
                        );
                        FactoryAccess::Unknown
                    }
                };
                ctx.emit(FactoryAccessModelEvent::Resolved);
            },
        ));
    }

    /// Resets to `Unknown` on logout or account change so the next authenticated session
    /// starts a fresh check. Aborts a still in-flight probe from the ending session so its
    /// response cannot land afterward and apply to the next session's access instead.
    pub fn reset(&mut self) {
        if let Some(probe) = self.probe.take() {
            probe.abort();
        }
        self.access = FactoryAccess::Unknown;
        self.requested = false;
    }
}

impl Entity for FactoryAccessModel {
    type Event = FactoryAccessModelEvent;
}

impl SingletonEntity for FactoryAccessModel {}

#[cfg(test)]
#[path = "factory_access_tests.rs"]
mod tests;
