//! Server metrics, against a live database (ADR 0026).
//!
//! Four kinds of claim. The names are a contract: what was published stays,
//! with its type and its labels, and the reference page lists exactly that.
//! Only an administrator's token of scope `metrics` reads them, and only while
//! its owner is one. A request is counted by the pattern of its route, never by
//! the path - no series names a person. And each figure comes from where it
//! lives: the queue from its rows, a task's runs from the task itself.

mod support;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::http::{StatusCode, header};
use kasl_server::{
    metrics::{Recorder, Task},
    webhooks::{Destination, Webhooks},
};
use serde_json::json;
use support::TestServer;

// The published figures ------------------------------------------------------------
//
// What v0.30 published, copied here rather than read from the server: a figure
// renamed in the code must fail a test, not quietly update the promise. The
// name is the one a query uses - a counter's with its `_total`.

const PUBLISHED: &[(&str, &str, &[&str])] = &[
    ("kasl_build_info", "gauge", &["version"]),
    ("kasl_http_requests_total", "counter", &["method", "route", "status"]),
    ("kasl_http_request_duration_seconds", "histogram", &["method", "route"]),
    ("kasl_background_runs_total", "counter", &["task", "outcome"]),
    ("kasl_background_last_success_timestamp_seconds", "gauge", &["task"]),
    ("kasl_database_connections", "gauge", &["state"]),
    ("kasl_database_connections_max", "gauge", &[]),
    ("kasl_database_round_trip_seconds", "gauge", &[]),
    ("kasl_webhook_deliveries", "gauge", &["destination", "state"]),
    ("kasl_webhook_oldest_pending_age_seconds", "gauge", &["destination"]),
    ("process_start_time_seconds", "gauge", &[]),
    ("process_cpu_seconds_total", "counter", &[]),
    ("process_resident_memory_bytes", "gauge", &[]),
    ("process_virtual_memory_bytes", "gauge", &[]),
    ("process_threads", "gauge", &[]),
    ("process_open_fds", "gauge", &[]),
    ("process_max_fds", "gauge", &[]),
];

/// Read from `/proc`, so on Linux alone.
const LINUX_ONLY: &[&str] = &[
    "process_cpu_seconds_total",
    "process_resident_memory_bytes",
    "process_virtual_memory_bytes",
    "process_threads",
    "process_open_fds",
    "process_max_fds",
];

/// Absent where the system sets no limit on open files.
const WHEN_LIMITED: &str = "process_max_fds";

/// The family a query name belongs to, as the `# TYPE` line names it.
fn family(name: &str, kind: &str) -> String {
    match kind {
        "counter" => name.strip_suffix("_total").expect("a counter is queried with `_total`").to_string(),
        _ => name.to_string(),
    }
}

// Reading a scrape -----------------------------------------------------------------

/// One sample line: its name, its labels, its value.
#[derive(Debug)]
struct Sample {
    name: String,
    labels: BTreeMap<String, String>,
    value: f64,
}

struct Scrape {
    text: String,
}

impl Scrape {
    /// Every `# TYPE` line, family to type.
    fn types(&self) -> Vec<(String, String)> {
        self.text
            .lines()
            .filter_map(|line| line.strip_prefix("# TYPE "))
            .map(|rest| {
                let (name, kind) = rest.split_once(' ').expect("a TYPE line names a family and a type");
                (name.to_string(), kind.to_string())
            })
            .collect()
    }

    fn samples(&self) -> Vec<Sample> {
        self.text
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(parse_sample)
            .collect()
    }

    /// The samples of one family: a counter's `_total`, a histogram's buckets,
    /// sum and count.
    fn of_family(&self, family: &str, kind: &str) -> Vec<Sample> {
        let names: Vec<String> = match kind {
            "counter" => vec![format!("{family}_total")],
            "histogram" => vec![format!("{family}_bucket"), format!("{family}_sum"), format!("{family}_count")],
            _ => vec![family.to_string()],
        };
        self.samples().into_iter().filter(|sample| names.contains(&sample.name)).collect()
    }

    /// The value of the one sample with exactly these labels.
    fn value(&self, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        let wanted: BTreeMap<String, String> = labels.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
        self.samples()
            .into_iter()
            .find(|sample| sample.name == name && sample.labels == wanted)
            .map(|sample| sample.value)
    }
}

/// `name{key="value",...} 1.5` or `name 1.5`. A value may hold braces - a
/// route pattern does - but never a quote, which is all this has to know.
fn parse_sample(line: &str) -> Sample {
    let (head, value) = line.rsplit_once(' ').unwrap_or_else(|| panic!("a sample has a value: {line}"));
    let value: f64 = value.parse().unwrap_or_else(|_| panic!("a sample's value is a number: {line}"));
    let Some((name, rest)) = head.split_once('{') else {
        return Sample {
            name: head.to_string(),
            labels: BTreeMap::new(),
            value,
        };
    };
    let body = rest.strip_suffix('}').unwrap_or_else(|| panic!("labels close with a brace: {line}"));
    let mut labels = BTreeMap::new();
    let mut rest = body;
    while !rest.is_empty() {
        let (key, after) = rest.split_once("=\"").unwrap_or_else(|| panic!("a label is key=\"value\": {line}"));
        let (label, after) = after.split_once('"').unwrap_or_else(|| panic!("a label value closes: {line}"));
        labels.insert(key.to_string(), label.to_string());
        rest = after.strip_prefix(',').unwrap_or(after);
    }
    Sample {
        name: name.to_string(),
        labels,
        value,
    }
}

// The cast -------------------------------------------------------------------------

/// Two administrators - one issues, the other can change them - and a manager.
struct Cast {
    issuer: Option<String>,
    other_admin: Option<String>,
    manager: Option<String>,
    issuer_id: String,
    manager_id: String,
}

async fn cast(server: &TestServer) -> Cast {
    server.add_admin("boss@example.test", "correct horse").await;
    server.add_admin("root@example.test", "correct horse").await;
    let (_, issuer, _) = server.login("boss@example.test", "correct horse").await;
    let (_, other_admin, _) = server.login("root@example.test", "correct horse").await;

    server.add_agent("lead@example.test", "lead-token").await;
    server.set_password("lead@example.test", "correct horse").await;
    let lead_id = user_id(server, other_admin.as_deref(), "lead@example.test").await;
    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{lead_id}"), other_admin.as_deref(), json!({ "role": "manager" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;

    let issuer_id = user_id(server, other_admin.as_deref(), "boss@example.test").await;
    Cast {
        issuer,
        other_admin,
        manager,
        issuer_id,
        manager_id: lead_id,
    }
}

async fn user_id(server: &TestServer, admin: Option<&str>, email: &str) -> String {
    let (status, users) = server.get_with_cookie("/api/v1/users", admin).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    users
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["email"] == email)
        .and_then(|user| user["id"].as_str())
        .expect("the account should exist")
        .to_string()
}

/// Issues a token of `scope` to whoever holds `cookie`: its id and the token.
async fn issue(server: &TestServer, cookie: Option<&str>, scope: &str) -> (String, String) {
    let (status, _, body) = server
        .post_with_cookie("/api/v1/read-tokens", cookie, json!({ "name": "Prometheus", "scope": scope }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["scope"], scope);
    (body["id"].as_str().unwrap().to_string(), body["token"].as_str().unwrap().to_string())
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Scrapes with `token`, insisting the scrape is a whole OpenMetrics document.
async fn scrape(server: &TestServer, token: &str) -> Scrape {
    let (status, headers, bytes) = server.get_file_with_header("/metrics", Some(&bearer(token))).await;
    let text = String::from_utf8(bytes).expect("the scrape is UTF-8");
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()),
        Some("application/openmetrics-text; version=1.0.0; charset=utf-8")
    );
    assert!(
        text.ends_with("# EOF\n"),
        "a scrape ends with `# EOF`, which is how Prometheus knows it arrived whole"
    );
    Scrape { text }
}

async fn status_of(server: &TestServer, path: &str, token: &str) -> StatusCode {
    server.get_with_header(path, Some(&bearer(token))).await.0
}

// The contract ---------------------------------------------------------------------

#[tokio::test]
async fn what_was_published_is_there_with_its_type_and_labels() {
    let Some(server) = TestServer::start().await else { return };
    let destination = Destination::parse("KASL_WEBHOOK_TEAM", "slack http://127.0.0.1:1/services/T0/B0/HOOKSECRET").unwrap();
    let server = server.with_webhooks(Webhooks::new(vec![destination], None));
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;

    // Something for the request figures to have counted.
    server.get_with_header("/health", None).await;
    let scrape = scrape(&server, &token).await;

    let types = scrape.types();
    let mut families: Vec<&String> = types.iter().map(|(family, _)| family).collect();
    families.sort();
    let before = families.len();
    families.dedup();
    assert_eq!(families.len(), before, "no family is written twice: {types:?}");

    for (name, kind, labels) in PUBLISHED {
        if LINUX_ONLY.contains(name) && !cfg!(target_os = "linux") {
            continue;
        }
        let family = family(name, kind);
        let Some((_, written)) = types.iter().find(|(written, _)| *written == family) else {
            assert!(*name == WHEN_LIMITED, "`{name}` was published and is missing from the scrape:\n{}", scrape.text);
            continue;
        };
        assert_eq!(written, kind, "`{name}` keeps its type");

        let samples = scrape.of_family(&family, kind);
        assert!(!samples.is_empty(), "`{name}` has something to say in this scrape:\n{}", scrape.text);
        for sample in samples {
            let mut keys: Vec<&str> = sample.labels.keys().map(String::as_str).filter(|key| *key != "le").collect();
            keys.sort_unstable();
            let mut published = labels.to_vec();
            published.sort_unstable();
            assert_eq!(keys, published, "`{name}` keeps its labels: {sample:?}");
        }
    }

    assert_eq!(scrape.value("kasl_build_info", &[("version", env!("CARGO_PKG_VERSION"))]), Some(1.0));
}

#[test]
fn the_reference_lists_every_figure_in_order() {
    // The table under "The figures": a backticked name, its type, and its
    // labels backticked and comma-separated. Read from the table rows only, so
    // prose that mentions a figure neither satisfies nor breaks the check.
    let page = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/src/content/docs/reference/server-metrics.md"))
        .expect("the metrics reference page should exist");
    let heading = "## The figures";
    let start = page.find(heading).expect("the page has a `The figures` section");
    let section = &page[start + heading.len()..];
    let section = &section[..section.find("\n#").unwrap_or(section.len())];

    let listed: Vec<(String, String, Vec<String>)> = section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            let name = cells[0].trim_matches('`').to_string();
            let labels = cells[2]
                .split(',')
                .map(|label| label.trim().trim_matches('`').to_string())
                .filter(|label| !label.is_empty())
                .collect();
            (name, cells[1].to_string(), labels)
        })
        .collect();

    let published: Vec<(String, String, Vec<String>)> = PUBLISHED
        .iter()
        .map(|(name, kind, labels)| (name.to_string(), kind.to_string(), labels.iter().map(|label| label.to_string()).collect()))
        .collect();
    assert_eq!(listed, published, "the figures table on the reference page");
}

// Who reads them -------------------------------------------------------------------

#[tokio::test]
async fn only_an_administrator_holds_a_metrics_token_and_only_while_they_are_one() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    // A manager reads the hours with a token, never the server.
    let (status, _, body) = server
        .post_with_cookie(
            "/api/v1/read-tokens",
            cast.manager.as_deref(),
            json!({ "name": "Prometheus", "scope": "metrics" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // A token names what it reads; there is no scope by default.
    let (status, _, body) = server
        .post_with_cookie("/api/v1/read-tokens", cast.issuer.as_deref(), json!({ "name": "Prometheus" }))
        .await;
    assert!(status.is_client_error(), "a token without a scope is refused: {status} {body}");

    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;
    assert_eq!(status_of(&server, "/metrics", &token).await, StatusCode::OK);
    let scope: String = server
        .scalar("SELECT details->>'scope' FROM audit_log WHERE action = 'read_token.issued'")
        .await;
    assert_eq!(scope, "metrics", "the audit log says what the token reads");

    // Made a manager: the metrics are an administrator's, so the token stops.
    let (status, _, body) = server
        .patch_with_cookie(
            &format!("/api/v1/users/{}", cast.issuer_id),
            cast.other_admin.as_deref(),
            json!({ "role": "manager" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(status_of(&server, "/metrics", &token).await, StatusCode::FORBIDDEN);

    // An administrator again, it reads again: authority is read per request.
    let (status, _, _) = server
        .patch_with_cookie(
            &format!("/api/v1/users/{}", cast.issuer_id),
            cast.other_admin.as_deref(),
            json!({ "role": "admin" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(status_of(&server, "/metrics", &token).await, StatusCode::OK);

    // Deactivated: as dead as a token that never existed.
    let (status, _, _) = server
        .patch_with_cookie(
            &format!("/api/v1/users/{}", cast.issuer_id),
            cast.other_admin.as_deref(),
            json!({ "active": false }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(status_of(&server, "/metrics", &token).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_two_scopes_open_different_doors() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, metrics) = issue(&server, cast.issuer.as_deref(), "metrics").await;
    let (_, hours) = issue(&server, cast.issuer.as_deref(), "hours").await;

    assert_eq!(status_of(&server, "/metrics", &hours).await, StatusCode::FORBIDDEN);
    assert_eq!(status_of(&server, "/api/bi/v1/people", &metrics).await, StatusCode::FORBIDDEN);
    assert_eq!(status_of(&server, "/api/bi/v1/people", &hours).await, StatusCode::OK);

    // Nobody without a token, and not an agent's.
    let (status, _) = server.get_with_header("/metrics", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(status_of(&server, "/metrics", &server.token).await, StatusCode::UNAUTHORIZED);
    let (status, _) = server.get_with_cookie("/metrics", cast.issuer.as_deref()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a session is not a bearer token");
}

#[tokio::test]
async fn a_metrics_token_reads_nobody_so_the_privacy_manifest_does_not_list_it() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    issue(&server, cast.issuer.as_deref(), "metrics").await;
    let (status, manifest) = server.get_with_cookie("/api/v1/privacy", cast.manager.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{manifest}");
    assert_eq!(manifest["read_elsewhere"], json!([]), "a metrics token reads no person's data");

    // The same administrator's hours token does, and is listed.
    issue(&server, cast.issuer.as_deref(), "hours").await;
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cast.manager.as_deref()).await;
    let listed = manifest["read_elsewhere"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{manifest}");
    assert_eq!(listed[0]["about"], "everyone");

    // A manager's reaches their department...
    let (status, _, body) = server
        .post_with_cookie(
            "/api/v1/departments",
            cast.other_admin.as_deref(),
            json!({ "name": "Engineering", "manager_id": cast.manager_id }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    issue(&server, cast.manager.as_deref(), "hours").await;
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cast.other_admin.as_deref()).await;
    assert_eq!(manifest["read_elsewhere"].as_array().unwrap().len(), 2, "{manifest}");

    // ...until they are made an employee. The department still names them,
    // but the token reads nothing now, and the manifest stops saying it does
    // - by the rule the token itself is held to.
    let (status, _, body) = server
        .patch_with_cookie(
            &format!("/api/v1/users/{}", cast.manager_id),
            cast.other_admin.as_deref(),
            json!({ "role": "employee" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cast.other_admin.as_deref()).await;
    let listed = manifest["read_elsewhere"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{manifest}");
    assert_eq!(listed[0]["about"], "everyone", "only the administrator's is left");
}

// What is counted ------------------------------------------------------------------

#[tokio::test]
async fn a_request_is_counted_by_its_route_and_never_by_its_path() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;
    let issuer_id = cast.issuer_id.clone();

    for _ in 0..2 {
        let (status, _) = server
            .get_with_cookie(
                &format!("/api/v1/users/{issuer_id}/days?from=2026-08-24&to=2026-08-24"),
                cast.other_admin.as_deref(),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = server.get_with_cookie("/api/v1/no-such-thing", cast.other_admin.as_deref()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    server.get_with_cookie("/team/somebody", None).await;
    let (status, _) = server.post_with_header("/api/v1/days", None, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let scrape = scrape(&server, &token).await;
    let route = "/api/v1/users/{id}/days";
    assert_eq!(
        scrape.value("kasl_http_requests_total", &[("method", "GET"), ("route", route), ("status", "200")]),
        Some(2.0),
        "{}",
        scrape.text
    );
    assert_eq!(
        scrape.value("kasl_http_request_duration_seconds_count", &[("method", "GET"), ("route", route)]),
        Some(2.0)
    );
    assert_eq!(
        scrape.value("kasl_http_requests_total", &[("method", "GET"), ("route", "unmatched"), ("status", "404")]),
        Some(1.0)
    );
    assert!(
        scrape
            .samples()
            .iter()
            .any(|sample| sample.name == "kasl_http_requests_total" && sample.labels.get("route").map(String::as_str) == Some("web")),
        "a page of the web UI is counted as `web`"
    );
    assert_eq!(
        scrape.value("kasl_http_requests_total", &[("method", "POST"), ("route", "/api/v1/days"), ("status", "401")]),
        Some(1.0),
        "a refusal is counted under the route that refused it"
    );
    assert!(!scrape.text.contains(&issuer_id), "no series names a person");
    assert!(
        !scrape.text.contains("somebody") && !scrape.text.contains("no-such-thing"),
        "nor a path somebody typed"
    );
}

#[tokio::test]
async fn the_webhook_queue_is_read_from_its_rows() {
    let Some(server) = TestServer::start().await else { return };
    let team = Destination::parse("KASL_WEBHOOK_TEAM", "slack http://127.0.0.1:1/services/T0/B0/HOOKSECRET").unwrap();
    let quiet = Destination::parse("KASL_WEBHOOK_QUIET", "slack http://127.0.0.1:1/services/T0/B0/OTHERSECRET").unwrap();
    let server = server.with_webhooks(Webhooks::new(vec![team, quiet], None));
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;

    // Three waiting - the oldest two hours - two delivered, one given up on,
    // and one for a destination no longer configured. Every count differs, so
    // a state read as another cannot pass.
    server
        .execute(
            "INSERT INTO webhook_deliveries (event_id, destination, event, payload, created_at, next_attempt_at, delivered_at, abandoned_at)
             VALUES
               (gen_random_uuid(), 'team', 'test', '{}', now() - interval '2 hours', now(), NULL, NULL),
               (gen_random_uuid(), 'team', 'test', '{}', now() - interval '10 minutes', now(), NULL, NULL),
               (gen_random_uuid(), 'team', 'test', '{}', now() - interval '1 minute', now(), NULL, NULL),
               (gen_random_uuid(), 'team', 'test', '{}', now() - interval '3 hours', now(), now() - interval '3 hours', NULL),
               (gen_random_uuid(), 'team', 'test', '{}', now() - interval '3 hours', now(), now() - interval '3 hours', NULL),
               (gen_random_uuid(), 'team', 'test', '{}', now() - interval '5 hours', now(), NULL, now() - interval '4 hours'),
               (gen_random_uuid(), 'gone', 'test', '{}', now() - interval '9 hours', now(), NULL, NULL)",
        )
        .await;

    let scrape = scrape(&server, &token).await;
    let deliveries = |destination: &str, state: &str| scrape.value("kasl_webhook_deliveries", &[("destination", destination), ("state", state)]);
    assert_eq!(deliveries("team", "pending"), Some(3.0), "{}", scrape.text);
    assert_eq!(deliveries("team", "delivered"), Some(2.0));
    assert_eq!(deliveries("team", "abandoned"), Some(1.0));

    let oldest = scrape
        .value("kasl_webhook_oldest_pending_age_seconds", &[("destination", "team")])
        .expect("the oldest pending delivery's age");
    assert!((7200.0..7300.0).contains(&oldest), "the oldest of the three waited two hours: {oldest}");

    // A destination with nothing in its queue still says so, at zero.
    assert_eq!(deliveries("quiet", "pending"), Some(0.0));
    assert_eq!(scrape.value("kasl_webhook_oldest_pending_age_seconds", &[("destination", "quiet")]), Some(0.0));
    // One no longer configured is the dispatcher's to give up on, not a series.
    assert!(!scrape.text.contains("destination=\"gone\""), "{}", scrape.text);
}

#[tokio::test]
async fn the_timed_tasks_report_their_runs() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;
    let recorder = server.recorder();
    let webhooks = Arc::new(server.webhooks().clone());

    let started = scrape(&server, &token).await;
    let start = started
        .value("kasl_background_last_success_timestamp_seconds", &[("task", "alert_sweep")])
        .expect("a task's last success exists from the start");
    for task in ["alert_sweep", "webhook_dispatch"] {
        for outcome in ["ok", "error"] {
            assert_eq!(
                started.value("kasl_background_runs_total", &[("task", task), ("outcome", outcome)]),
                Some(0.0),
                "`{task}` {outcome} exists at zero before the first run"
            );
        }
    }

    // The loops the server starts, started here with this server's recorder.
    // Both run once at once, then on their timers.
    kasl_server::alerts::run_sweeps(server.pool.clone(), webhooks.clone(), recorder.clone());
    kasl_server::webhooks::run_dispatcher(server.pool.clone(), webhooks, recorder.clone());

    let mut ran = None;
    for _ in 0..100 {
        let now = scrape(&server, &token).await;
        let sweeps = now.value("kasl_background_runs_total", &[("task", "alert_sweep"), ("outcome", "ok")]);
        let dispatches = now.value("kasl_background_runs_total", &[("task", "webhook_dispatch"), ("outcome", "ok")]);
        if sweeps >= Some(1.0) && dispatches >= Some(1.0) {
            ran = Some(now);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ran = ran.expect("both tasks report a run within five seconds of starting");
    let last = ran.value("kasl_background_last_success_timestamp_seconds", &[("task", "alert_sweep")]).unwrap();
    assert!(last > start, "a completed run moves the task's last success: {start} -> {last}");
}

#[tokio::test]
async fn a_failed_run_is_counted_and_leaves_the_last_success_where_it_was() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;
    let recorder: Arc<Recorder> = server.recorder();

    recorder.record_run(Task::WebhookDispatch, true);
    let before = scrape(&server, &token)
        .await
        .value("kasl_background_last_success_timestamp_seconds", &[("task", "webhook_dispatch")])
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    recorder.record_run(Task::WebhookDispatch, false);
    recorder.record_run(Task::WebhookDispatch, false);

    let after = scrape(&server, &token).await;
    assert_eq!(
        after.value("kasl_background_runs_total", &[("task", "webhook_dispatch"), ("outcome", "error")]),
        Some(2.0)
    );
    assert_eq!(
        after.value("kasl_background_runs_total", &[("task", "webhook_dispatch"), ("outcome", "ok")]),
        Some(1.0)
    );
    assert_eq!(
        after.value("kasl_background_last_success_timestamp_seconds", &[("task", "webhook_dispatch")]),
        Some(before),
        "a failure is not a success"
    );
}

#[tokio::test]
async fn the_pool_says_how_many_connections_it_holds() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.issuer.as_deref(), "metrics").await;

    // Three connections held across the scrape, as three slow requests would.
    // The test pool opens four and the scrape's own queries take the last; sqlx
    // hands a connection back on a task of its own, so when the pool is
    // counted that one may or may not be idle yet. Three busy at least, and at
    // most one idle: an idle count of three is the busy one read as the other.
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(server.pool.acquire().await.expect("a connection to hold"));
    }
    let scrape = scrape(&server, &token).await;
    drop(held);

    let max = scrape.value("kasl_database_connections_max", &[]).expect("the pool's limit");
    assert_eq!(max, f64::from(server.pool.options().get_max_connections()));
    let idle = scrape.value("kasl_database_connections", &[("state", "idle")]).unwrap();
    let busy = scrape.value("kasl_database_connections", &[("state", "busy")]).unwrap();
    assert!(busy >= 3.0, "the three held are busy: {}", scrape.text);
    assert!(
        idle + busy <= max && idle <= max - 3.0,
        "the idle are what is left: {idle} idle, {busy} busy of {max}"
    );
    let round_trip = scrape.value("kasl_database_round_trip_seconds", &[]).expect("a round trip was measured");
    assert!(round_trip > 0.0 && round_trip < 2.0, "{round_trip}");
}
