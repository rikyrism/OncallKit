use axum::{
    http::{header, HeaderName, HeaderValue},
    middleware,
    routing::{delete, get, patch, post, put},
    Router,
};
use tower_http::{
    limit::RequestBodyLimitLayer,
    propagate_header::PropagateHeaderLayer,
    request_id::{MakeRequestUuid, SetRequestIdLayer},
    sensitive_headers::SetSensitiveHeadersLayer,
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{audit, auth, calendar, integrations, notifications, observability, organizations, schedules, state::AppState, swaps, teams};

pub fn router(state: AppState) -> Router {
    let request_id = HeaderName::from_static("x-request-id");

    let api = Router::new()
        // auth
        .route("/auth/providers", get(auth::routes::providers))
        .route("/auth/google/start", get(auth::routes::google_start))
        .route("/auth/google/callback", get(auth::routes::google_callback))
        .route("/auth/dev/login", post(auth::routes::dev_login))
        .route("/auth/logout", post(auth::routes::logout))
        .route("/me", get(auth::routes::me).patch(auth::routes::update_profile))
        .route("/me/sessions", get(auth::routes::list_sessions))
        .route("/me/sessions/{id}", delete(auth::routes::revoke_session))
        .route("/me/invitations", get(organizations::my_invitations))
        .route("/invitations/accept", post(organizations::accept_invitation))
        // organizations & members
        .route("/organizations", get(organizations::list).post(organizations::create))
        .route("/organizations/current", patch(organizations::update_current))
        .route("/organizations/current/transfer-ownership", post(organizations::transfer_ownership))
        .route("/organizations/{id}", get(organizations::get))
        .route("/organizations/{id}/activate", post(organizations::activate))
        .route("/members", get(organizations::list_members))
        .route("/members/{user_id}", patch(organizations::update_member).delete(organizations::remove_member))
        .route("/members/{user_id}/discord", put(organizations::set_member_discord))
        .route("/invitations", get(organizations::list_invitations).post(organizations::create_invitation))
        .route("/invitations/{id}", delete(organizations::revoke_invitation))
        // teams
        .route("/teams", get(teams::list).post(teams::create))
        .route("/teams/{id}", get(teams::get).patch(teams::update).delete(teams::delete))
        // schedules
        .route("/oncall/overview", get(schedules::overview))
        .route("/schedules", get(schedules::list).post(schedules::create))
        .route("/schedules/preview", post(schedules::preview))
        .route("/schedules/{id}", get(schedules::get).patch(schedules::update).delete(schedules::archive))
        .route("/schedules/{id}/on-call-now", get(schedules::on_call))
        .route("/schedules/{id}/upcoming", get(schedules::upcoming))
        .route("/schedules/{id}/assignments", get(schedules::assignments))
        .route("/schedules/{id}/overrides", post(schedules::create_override))
        .route("/schedules/{id}/overrides/{override_id}", delete(schedules::cancel_override))
        // shift swaps
        .route("/swap-requests", get(swaps::list).post(swaps::create))
        .route("/swap-requests/{id}/accept", post(swaps::accept))
        .route("/swap-requests/{id}/decline", post(swaps::decline))
        .route("/swap-requests/{id}/cancel", post(swaps::cancel))
        // integrations & notifications
        .route("/integrations", get(integrations::list))
        .route("/integrations/events", get(integrations::event_options))
        .route("/integrations/discord", post(integrations::create_discord))
        .route("/integrations/{id}", patch(integrations::update).delete(integrations::delete))
        .route("/integrations/{id}/test", post(integrations::test))
        .route("/notifications/deliveries", get(notifications::routes::list_deliveries))
        .route("/notifications/deliveries/{id}/attempts", get(notifications::routes::delivery_attempts))
        .route("/notifications/deliveries/{id}/retry", post(notifications::routes::retry_delivery))
        .route("/notifications/health", get(notifications::routes::health))
        .route("/audit-logs", get(audit::list))
        // calendar subscriptions
        .route("/calendar-feeds", get(calendar::list).post(calendar::create))
        .route("/calendar-feeds/{id}", delete(calendar::revoke))
        .route("/calendar.ics", get(calendar::feed))
        .route("/openapi.yaml", get(openapi));

    Router::new()
        .nest("/api/v1", api)
        .route("/healthz", get(observability::healthz))
        .route("/readyz", get(observability::readyz))
        .route("/metrics", get(observability::metrics))
        .layer(middleware::from_fn_with_state(state.clone(), observability::track_http))
        .layer(RequestBodyLimitLayer::new(256 * 1024))
        .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::REQUEST_TIMEOUT, state.config.request_timeout))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("same-origin")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(PropagateHeaderLayer::new(request_id.clone()))
        .layer(TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
            let id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or("-");
            // Only the path is logged: query strings can carry OAuth codes.
            tracing::info_span!("http", method = %req.method(), path = %req.uri().path(), request_id = %id)
        }))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(SetSensitiveHeadersLayer::new([header::COOKIE, header::SET_COOKIE, header::AUTHORIZATION, HeaderName::from_static("x-csrf-token")]))
        .with_state(state)
}

async fn openapi() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::CONTENT_TYPE, "application/yaml")], include_str!("../../../docs/openapi.yaml"))
}
