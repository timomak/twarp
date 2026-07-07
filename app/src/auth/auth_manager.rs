use std::sync::Arc;

use twarpui::{Entity, ModelContext, SingletonEntity};

use super::auth_state::AuthState;
use super::AuthStateProvider;
// twarp: 2c-d — AI llms / persisted workspace / usage deleted; LLMPreferences re-exported.
pub use crate::terminal::input::LLMPreferences;
pub struct PersistedWorkspace;
impl twarpui::Entity for PersistedWorkspace {
    type Event = ();
}
impl twarpui::SingletonEntity for PersistedWorkspace {}
#[allow(dead_code)]
impl PersistedWorkspace {
    pub fn on_user_changed<C>(&mut self, _: &mut C) {}
}
pub struct AIRequestUsageModel;
impl twarpui::Entity for AIRequestUsageModel {
    type Event = ();
}
impl twarpui::SingletonEntity for AIRequestUsageModel {}
#[allow(dead_code)]
impl AIRequestUsageModel {
    pub fn refresh_request_usage_async<C>(&mut self, _: &mut C) {}
}

pub type LoginGatedFeature = &'static str;

type URLConstructorCallback = Box<dyn FnOnce(Option<&str>) -> String>;

/// AuthManager is a singleton model which manages the currently logged-in user's state.
/// If you need to access the state, use `AuthStateProvider`.
///
/// twarp: de-cloud — the login/signup UI, the Firebase auth client, and every
/// sign-in/sign-up/device-auth/anonymous-user flow were deleted. The manager is
/// kept as a type (many models reference it) but the user is permanently
/// logged out; nothing here talks to a server or emits auth events anymore.
pub struct AuthManager {
    auth_state: Arc<AuthState>,
}

impl AuthManager {
    /// Creates a new instance of the AuthManager. The auth state must already be initialized
    /// through [`AuthStateProvider`].
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        Self { auth_state }
    }

    #[cfg(test)]
    pub fn new_for_test(ctx: &mut ModelContext<Self>) -> Self {
        Self::new(ctx)
    }

    /// twarp: de-cloud — login is gone, so login-gated features are simply
    /// unavailable; log instead of showing the (deleted) login modal.
    pub fn attempt_login_gated_feature(
        &self,
        feature: LoginGatedFeature,
        _ctx: &mut ModelContext<Self>,
    ) {
        log::info!("Login-gated feature unavailable (logins are disabled): {feature}");
    }

    /// twarp: de-cloud — anonymous Firebase users no longer exist, so there is
    /// no object limit (or sign-up nudge) to surface.
    pub fn anonymous_user_hit_drive_object_limit(&self, _ctx: &mut ModelContext<Self>) {}

    /// Opens a page in the web app.
    /// twarp: de-cloud — the anonymous-user custom-token handoff was deleted;
    /// the URL is always opened without a login token.
    pub fn open_url_maybe_with_anonymous_token(
        &self,
        ctx: &mut ModelContext<Self>,
        construct_url: URLConstructorCallback,
    ) {
        let url: String = construct_url(None);
        ctx.open_url(&url);
    }

    /// Sets the user as onboarded locally.
    /// twarp: de-cloud — the server-side `set_user_is_onboarded` mutation and
    /// keychain persistence were deleted; only the in-memory flag remains.
    pub fn set_user_onboarded(&self, _ctx: &mut ModelContext<Self>) {
        self.auth_state.set_is_onboarded(true);
    }
}

#[derive(Clone, Debug)]
pub struct PersistedCurrentUserInformation {
    pub email: String,
}

impl Entity for AuthManager {
    type Event = ();
}

impl SingletonEntity for AuthManager {}
