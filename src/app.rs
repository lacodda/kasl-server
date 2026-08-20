use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use serde_json::json;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::{admin, config::Config, ingest, login};

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
        // Administration. Reading the team is a manager's; changing it is not,
        // until departments give a manager something to be in charge of.
        .route("/users", get(admin::list_users).post(admin::create_user))
        .route("/users/{id}", patch(admin::update_user))
        .route("/users/{id}/agents", get(admin::list_agents).post(admin::create_agent))
        .route("/agents/{id}", delete(admin::revoke_agent));

    Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api_v1)
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

/// Liveness + readiness in one place: the process answers, and the database
/// round-trip tells whether the server can actually do its job.
async fn health(State(state): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "database": "ok",
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
    async fn unknown_routes_return_404() {
        let response = router(dead_pool()).oneshot(Request::get("/nope").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
