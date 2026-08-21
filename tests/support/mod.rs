//! Shared scaffolding: a throwaway database, and a server wired to it.
//!
//! Each test gets a database of its own, migrated from empty. A shared one
//! would make the suite order-dependent - these tests assert on row counts,
//! uniqueness and cascades, which a neighbour's leftovers would break.

#![allow(dead_code)]

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::{AssertSqlSafe, Connection, Executor, PgConnection, PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

/// A migrated database that drops itself when the test ends.
pub struct TestDb {
    admin_url: String,
    name: String,
    pub pool: PgPool,
}

impl TestDb {
    /// Creates and migrates a database, or returns `None` when the environment
    /// offers no server to create one on.
    pub async fn create() -> Option<Self> {
        let admin_url = std::env::var("DATABASE_URL").ok()?;
        let name = format!("kasl_test_{}", Uuid::new_v4().simple());

        let mut admin = PgConnection::connect(&admin_url).await.expect("DATABASE_URL is set but not reachable");
        // `CREATE DATABASE` takes no bind parameters, so the name is
        // interpolated. It is a literal prefix plus a generated UUID, never
        // anything from outside this test.
        admin
            .execute(AssertSqlSafe(format!(r#"CREATE DATABASE "{name}""#)))
            .await
            .expect("failed to create the test database");
        admin.close().await.ok();

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&replace_database(&admin_url, &name))
            .await
            .expect("failed to connect to the test database");
        kasl_server::migrator().run(&pool).await.expect("migrations failed");

        Some(Self { admin_url, name, pool })
    }

    pub async fn drop(self) {
        let Self { admin_url, name, pool } = self;
        pool.close().await;
        if let Ok(mut admin) = PgConnection::connect(&admin_url).await {
            // WITH (FORCE) so a lingering connection cannot leave the database
            // behind on the developer's server.
            let _ = admin.execute(AssertSqlSafe(format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#))).await;
            admin.close().await.ok();
        }
    }
}

/// Keeps only `name=value` from a `Set-Cookie` value: what the browser sends
/// back, without the attributes that are instructions to it rather than data.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap_or(set_cookie).trim().to_string()
}

/// Reads a response into a status and a JSON body.
async fn read_response(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response.into_body().collect().await.expect("the body should read").to_bytes();
    // An empty body is a legitimate answer; represent it as JSON null so
    // callers can index into it without branching.
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };

    (status, body)
}

/// Swaps the database name in a connection string, keeping credentials, host
/// and query parameters (`sslmode` and friends) intact.
pub fn replace_database(url: &str, database: &str) -> String {
    let (prefix, rest) = url.split_once("://").expect("DATABASE_URL must be a URL");
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let query = path.split_once('?').map(|(_, q)| format!("?{q}")).unwrap_or_default();
    format!("{prefix}://{authority}/{database}{query}")
}

/// Runs `test` against a freshly migrated database, or skips when there is none.
pub async fn with_db<F, Fut>(test: F)
where
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = ()>,
{
    let Some(db) = TestDb::create().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };
    let pool = db.pool.clone();
    test(pool).await;
    db.drop().await;
}

/// A database plus one provisioned agent, driven through the real router.
///
/// Requests go through `Router::oneshot` rather than a bound socket: the same
/// extractors, handlers and error mapping run, without a port to collide on.
pub struct TestServer {
    pub pool: PgPool,
    /// A working token for the seeded agent.
    pub token: String,
    db: Option<TestDb>,
}

impl TestServer {
    /// Starts a server with one agent, or returns `None` without a database.
    ///
    /// The skip is announced rather than silent: a test that quietly does
    /// nothing still reports "ok", and CI greps for this line to tell the two
    /// apart.
    pub async fn start() -> Option<Self> {
        let Some(db) = TestDb::create().await else {
            eprintln!("skipped: DATABASE_URL is not set");
            return None;
        };
        let pool = db.pool.clone();

        let server = Self {
            pool,
            token: "test-agent-token".to_string(),
            db: Some(db),
        };
        server.provision("employee@example.test", &server.token).await;
        Some(server)
    }

    /// Adds another agent and returns its token.
    pub async fn add_agent(&self, email: &str, token: &str) -> String {
        self.provision(email, token).await;
        token.to_string()
    }

    async fn provision(&self, email: &str, token: &str) {
        // Reuses the production path rather than inserting rows by hand: if
        // provisioning breaks, these tests should notice.
        let parsed = kasl_server::provision::parse_seeds(&format!("{email}:{token}")).expect("the seed fixture should parse");
        kasl_server::provision::apply_seeds(&self.pool, &parsed)
            .await
            .expect("provisioning should succeed");
    }

    /// Posts a day with a bearer token.
    pub async fn post_day(&self, token: &str, day: Value) -> (StatusCode, Value) {
        self.post_day_with_header(Some(&format!("Bearer {token}")), day).await
    }

    /// Posts a batch of days.
    pub async fn post_batch(&self, token: &str, days: Value) -> (StatusCode, Value) {
        self.post_to("/api/v1/days/batch", Some(&format!("Bearer {token}")), serde_json::json!({ "days": days }))
            .await
    }

    /// Posts a batch to a server built with the operator's own limits, which is
    /// the only way to exercise a limit without waiting for the default.
    pub async fn post_batch_with_limits(&self, token: &str, days: Value, config: &kasl_server::config::Config) -> (StatusCode, Value) {
        let request = Request::post("/api/v1/days/batch")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({ "days": days }).to_string()))
            .expect("the request should build");

        let response = kasl_server::app::router_with(self.pool.clone(), config)
            .oneshot(request)
            .await
            .expect("the router should answer");
        read_response(response).await
    }

    /// Posts a day with an arbitrary (or absent) Authorization header.
    pub async fn post_day_with_header(&self, authorization: Option<&str>, day: Value) -> (StatusCode, Value) {
        self.post_to("/api/v1/days", authorization, day).await
    }

    async fn post_to(&self, path: &str, authorization: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = authorization {
            request = request.header(header::AUTHORIZATION, value);
        }
        let request = request.body(Body::from(body.to_string())).expect("the request should build");

        let response = kasl_server::app::router(self.pool.clone())
            .oneshot(request)
            .await
            .expect("the router should answer");
        read_response(response).await
    }

    /// Creates an administrator with a password.
    pub async fn add_admin(&self, email: &str, password: &str) {
        kasl_server::provision::ensure_admin(&self.pool, email, password)
            .await
            .expect("the admin fixture should be created");
    }

    /// Gives an existing user a password without changing their role.
    pub async fn set_password(&self, email: &str, password: &str) {
        let hash = kasl_server::session::hash_password(password).expect("hashing should succeed");
        sqlx::query("UPDATE users SET password_hash = $1 WHERE lower(email) = lower($2)")
            .bind(hash)
            .bind(email)
            .execute(&self.pool)
            .await
            .expect("the password fixture should apply");
    }

    /// Signs in, returning the status, the `Set-Cookie` value, and the body.
    pub async fn login(&self, email: &str, password: &str) -> (StatusCode, Option<String>, Value) {
        let request = Request::post("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"email": email, "password": password}).to_string()))
            .expect("the request should build");

        let response = kasl_server::app::router(self.pool.clone())
            .oneshot(request)
            .await
            .expect("the router should answer");
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let (status, body) = read_response(response).await;
        (status, cookie, body)
    }

    /// GETs a path carrying a cookie, which may be a full `Set-Cookie` value -
    /// the name=value prefix is what a browser would send back.
    pub async fn get_with_cookie(&self, path: &str, cookie: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::get(path);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie_pair(cookie));
        }
        let request = request.body(Body::empty()).expect("the request should build");
        read_response(
            kasl_server::app::router(self.pool.clone())
                .oneshot(request)
                .await
                .expect("the router should answer"),
        )
        .await
    }

    /// GETs a path with an arbitrary Authorization header.
    pub async fn get_with_header(&self, path: &str, authorization: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::get(path);
        if let Some(value) = authorization {
            request = request.header(header::AUTHORIZATION, value);
        }
        let request = request.body(Body::empty()).expect("the request should build");
        read_response(
            kasl_server::app::router(self.pool.clone())
                .oneshot(request)
                .await
                .expect("the router should answer"),
        )
        .await
    }

    /// POSTs carrying a cookie, returning the status, any `Set-Cookie`, and the body.
    pub async fn post_with_cookie(&self, path: &str, cookie: Option<&str>, body: Value) -> (StatusCode, Option<String>, Value) {
        let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie_pair(cookie));
        }
        let request = request.body(Body::from(body.to_string())).expect("the request should build");

        let response = kasl_server::app::router(self.pool.clone())
            .oneshot(request)
            .await
            .expect("the router should answer");
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let (status, body) = read_response(response).await;
        (status, set_cookie, body)
    }

    /// PATCHes carrying a cookie.
    pub async fn patch_with_cookie(&self, path: &str, cookie: Option<&str>, body: Value) -> (StatusCode, Option<String>, Value) {
        self.send_with_cookie("PATCH", path, cookie, Some(body)).await
    }

    /// PUTs carrying a cookie.
    pub async fn put_with_cookie(&self, path: &str, cookie: Option<&str>, body: Value) -> (StatusCode, Option<String>, Value) {
        self.send_with_cookie("PUT", path, cookie, Some(body)).await
    }

    /// DELETEs carrying a cookie.
    pub async fn delete_with_cookie(&self, path: &str, cookie: Option<&str>) -> (StatusCode, Option<String>, Value) {
        self.send_with_cookie("DELETE", path, cookie, None).await
    }

    async fn send_with_cookie(&self, method: &str, path: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, Option<String>, Value) {
        let mut request = Request::builder().method(method).uri(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie_pair(cookie));
        }
        let request = request
            .body(body.map(|body| Body::from(body.to_string())).unwrap_or_else(Body::empty))
            .expect("the request should build");

        let response = kasl_server::app::router(self.pool.clone())
            .oneshot(request)
            .await
            .expect("the router should answer");
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let (status, body) = read_response(response).await;
        (status, set_cookie, body)
    }

    /// Posts a day carrying a session cookie instead of a bearer token.
    pub async fn post_day_with_cookie(&self, cookie: Option<&str>, day: Value) -> (StatusCode, Value) {
        let mut request = Request::post("/api/v1/days").header(header::CONTENT_TYPE, "application/json");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie_pair(cookie));
        }
        let request = request.body(Body::from(day.to_string())).expect("the request should build");
        read_response(
            kasl_server::app::router(self.pool.clone())
                .oneshot(request)
                .await
                .expect("the router should answer"),
        )
        .await
    }

    pub async fn count(&self, table: &str) -> i64 {
        let sql = match table {
            "workdays" => "SELECT count(*) FROM workdays",
            "pauses" => "SELECT count(*) FROM pauses",
            "tasks" => "SELECT count(*) FROM tasks",
            "agents" => "SELECT count(*) FROM agents",
            "sessions" => "SELECT count(*) FROM sessions",
            "users" => "SELECT count(*) FROM users",
            other => panic!("no counter for `{other}`"),
        };
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.expect("failed to count rows")
    }

    /// Reads a single value; the query must return exactly one row.
    pub async fn scalar<T>(&self, sql: &'static str) -> T
    where
        T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
    {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.expect("failed to read a value")
    }

    /// Reads a single nullable value.
    pub async fn optional_scalar<T>(&self, sql: &'static str) -> Option<T>
    where
        T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
    {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.expect("failed to read a value")
    }

    pub async fn execute(&self, sql: &'static str) {
        sqlx::query(sql).execute(&self.pool).await.expect("failed to execute");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // The database outlives the test only if the process dies mid-run; a
        // blocking drop here would need a runtime, so the cleanup is spawned.
        if let Some(db) = self.db.take()
            && let Ok(handle) = tokio::runtime::Handle::try_current()
        {
            handle.spawn(db.drop());
        }
    }
}
