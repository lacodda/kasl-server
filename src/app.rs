use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use serde_json::json;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::{admin, audit, auth, config::Config, demo, department, ingest, login, me, privacy, team, web};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Days one batch may carry; enforced by the batch handler.
    pub max_batch_days: usize,
    /// Whether session cookies carry `Secure`.
    pub secure_cookies: bool,
}

/// Builds the router with the operator's limits applied.
pub fn router_with(pool: PgPool, config: &Config) -> Router {
    // `/api/v1` from the very first endpoint: kasl agents update on their own
    // schedule, so the path a working agent calls must keep meaning what it
    // meant when that agent shipped (ADR 0001).
    let api_v1 = Router::new()
        .route("/days", post(ingest::upload_day))
        .route("/days/batch", post(ingest::upload_batch))
        // People, not agents: these carry a session cookie rather than a
        // bearer token, and the two never mix.
        .route("/auth/login", post(login::login))
        .route("/auth/logout", post(login::logout))
        .route("/auth/logout-everywhere", post(login::logout_everywhere))
        .route("/auth/me", get(login::me))
        .route("/auth/password", post(admin::change_own_password))
        // What a person can read about themselves. `/me` rather than their own
        // id under `/users`: this route consults no role and no department, so
        // there is no permission here to get wrong.
        .route("/me/days", get(me::days))
        // Other people's data, for whoever is entitled to it. Separate routes
        // from `/me` on purpose: here a permission is checked, and a route that
        // sometimes checks one is a route where forgetting is invisible.
        .route("/team/days", get(team::days))
        .route("/users/{id}/days", get(team::user_days))
        // Administration. Reading the team is a manager's; changing it is not,
        // until departments give a manager something to be in charge of.
        .route("/users", get(admin::list_users).post(admin::create_user))
        .route("/users/{id}", patch(admin::update_user))
        .route("/users/{id}/agents", get(admin::list_agents).post(admin::create_agent))
        .route("/agents/{id}", delete(admin::revoke_agent))
        // Departments: what gives a manager a boundary to be in charge of.
        .route("/departments", get(department::list).post(department::create))
        .route("/departments/{id}", patch(department::update).delete(department::delete))
        .route("/users/{id}/department", put(department::assign))
        // The record of who did what. Administrators only, and no way to
        // delete from it (ADR 0010).
        .route("/audit", get(audit::list))
        // What this installation stores about a person. Readable by anyone
        // signed in; set by an administrator alone (ADR 0011).
        .route("/privacy", get(privacy::show).put(privacy::update))
        // The same manifest for an agent's bearer token, so kasl can show it
        // in the CLI - where the employee already is - rather than requiring a
        // login to the server that watches them.
        .route("/privacy/agent", get(privacy::show_to_agent))
        // Whose token this is. The one question an agent can ask about
        // itself, and the one `kasl server connect` needs so a token pasted
        // from the wrong place is caught by a person rather than discovered
        // in a dashboard weeks later.
        .route("/agent/whoami", get(auth::whoami))
        // Who a visitor may sign in as. Answered only on a demo - anywhere
        // else it is a 404, so no real installation lists its people to
        // someone who has not signed in (ADR 0013).
        .route("/demo/accounts", get(demo::accounts));

    Router::new()
        .route("/health", get(health))
        // Anything under `/api` that no route matched is a client's mistake and
        // has to look like one. Without this the web UI's fallback would catch
        // it and answer a misspelled endpoint with `200` and an HTML page -
        // which a kasl agent would read as success.
        .nest("/api", Router::new().nest("/v1", api_v1).fallback(unknown_endpoint))
        // The web UI, compiled into the binary. Last on purpose: it answers
        // everything the API did not claim, so a real endpoint always wins
        // over the single-page app's own routing (ADR 0012).
        .fallback(web::serve)
        .with_state(AppState {
            pool,
            max_batch_days: config.max_batch_days,
            secure_cookies: config.secure_cookies,
        })
        // A body larger than this is refused before it is buffered: backfilling
        // a year and attacking the server look identical up to the size.
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .layer(TraceLayer::new_for_http())
}

/// The router with default limits - what the tests and `/health` callers want
/// when the limits are not what is under test.
pub fn router(pool: PgPool) -> Router {
    router_with(pool, &Config::defaults_for_database(String::new()))
}

/// Answers a path under `/api` that no route matched.
///
/// JSON, like every other API failure: a client that parses our errors must
/// not have to special-case the one shape that says "no such endpoint".
async fn unknown_endpoint() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "no such endpoint" }))).into_response()
}

/// Liveness + readiness in one place: the process answers, and the database
/// round-trip tells whether the server can actually do its job.
///
/// The round-trip reads the demo flag rather than `SELECT 1`: the web UI asks
/// this endpoint before anyone signs in, and "is this a demo" is the one
/// fact it needs at that moment (ADR 0013).
async fn health(State(state): State<AppState>) -> Response {
    let demo: Result<bool, sqlx::Error> = sqlx::query_scalar("SELECT demo FROM settings WHERE singleton").fetch_one(&state.pool).await;
    match demo {
        Ok(demo) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "database": "ok",
                "demo": demo,
            })),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "health check: database unreachable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "status": "degraded",
                    "version": env!("CARGO_PKG_VERSION"),
                    "database": "unavailable",
                })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::*;

    /// A pool pointing nowhere: `connect_lazy` never dials until a query runs,
    /// so the router can be exercised without a live database.
    fn dead_pool() -> PgPool {
        PgPoolOptions::new()
            // Keep the failure fast: the default acquire timeout is 30 s.
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect_lazy("postgres://nobody:nowhere@127.0.0.1:1/kasl")
            .expect("lazy pool creation does not touch the network")
    }

    #[tokio::test]
    async fn health_reports_degraded_without_a_database() {
        let response = router(dead_pool()).oneshot(Request::get("/health").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["status"], "degraded");
        assert_eq!(body["database"], "unavailable");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn an_unknown_api_path_is_a_json_404() {
        // Under `/api` a path that matched nothing is a client's mistake. It
        // must not reach the web UI's fallback, which would answer `200` and
        // an HTML page - success, as far as a kasl agent can tell.
        let response = router(dead_pool())
            .oneshot(Request::get("/api/v1/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"], "no such endpoint");
    }

    #[tokio::test]
    async fn an_unknown_page_path_belongs_to_the_web_ui() {
        // Outside `/api` an unmatched path is a client-side route, and only the
        // app knows whether it exists. This used to be a flat 404; it changed
        // deliberately when the UI arrived (ADR 0012).
        let response = router(dead_pool()).oneshot(Request::get("/nope").body(Body::empty()).unwrap()).await.unwrap();

        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("text/html") || content_type.starts_with("text/plain"),
            "the web UI answers this path, got `{content_type}`",
        );
    }
}
