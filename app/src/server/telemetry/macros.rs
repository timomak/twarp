// twarp: de-cloud — the RudderStack pipeline is deleted; `ServerApi::send_telemetry_event`
// is a no-op that drops the event. These macros are kept so call sites compile unchanged.

/// Formerly sent a telemetry event immediately; now evaluates and drops the event.
#[macro_export]
macro_rules! send_telemetry_sync_from_ctx {
    ($event:expr, $ctx:expr) => {
        #[allow(unused_imports)]
        use twarp_core::telemetry::TelemetryEvent as _;
        let event = $event;
        if event.enablement_state().is_enabled() {
            let server_api =
                <$crate::server::server_api::ServerApiProvider as twarpui::SingletonEntity>::handle(
                    $ctx,
                )
                .as_ref($ctx)
                .get();
            let privacy_settings_snapshot =
                <$crate::settings::PrivacySettings as twarpui::SingletonEntity>::handle($ctx)
                    .as_ref($ctx)
                    .get_snapshot($ctx);
            let _ = $ctx.spawn(
                async move {
                    if let Err(error) = server_api
                        .send_telemetry_event(event, privacy_settings_snapshot)
                        .await
                    {
                        log::warn!("Error occurred with sending telemetry event: {}", error);
                    }
                },
                |_, _, _| {},
            );
        }
    };
}

/// Same as [`send_telemetry_sync_from_ctx`], but can be used when the caller only has
/// access to an [`App`] and not a `ViewContext`.
#[macro_export]
macro_rules! send_telemetry_sync_from_app_ctx {
    ($event:expr, $app_ctx:expr) => {
        #[allow(unused_imports)]
        use twarp_core::telemetry::TelemetryEvent as _;
        if $event.enablement_state().is_enabled() {
            let server_api =
                <$crate::server::server_api::ServerApiProvider as twarpui::SingletonEntity>::handle(
                    $app_ctx,
                )
                .as_ref($app_ctx)
                .get();
            let privacy_settings_snapshot =
                <$crate::settings::PrivacySettings as twarpui::SingletonEntity>::handle($app_ctx)
                    .as_ref($app_ctx)
                    .get_snapshot($app_ctx);
            $app_ctx
                .background_executor()
                .spawn(async move {
                    if let Err(error) = server_api
                        .send_telemetry_event($event, privacy_settings_snapshot)
                        .await
                    {
                        log::warn!("Error occurred with sending telemetry event: {error}");
                    }
                })
                .detach();
        }
    };
}

/// Same as [`send_telemetry_from_ctx`], except it can be called any time you have an
/// Arc<Background>. Events are recorded into the in-memory queue and dropped.
#[macro_export]
macro_rules! send_telemetry_on_executor {
    ($auth_state: expr, $event:expr, $executor:expr) => {
        #[allow(unused_imports)]
        use twarp_core::telemetry::TelemetryEvent as _;
        let event = $event;
        if event.enablement_state().is_enabled() {
            let user_id = $auth_state.user_id().map(|uid| uid.as_string());
            let anonymous_id = $auth_state.anonymous_id();
            twarpui::record_telemetry_on_executor!(
                user_id,
                anonymous_id,
                event.name().into(),
                event.payload(),
                event.contains_ugc(),
                $executor
            );
        }
    };
}
