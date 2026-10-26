//! What the server says about its own health, for Prometheus (ADR 0026).
//!
//! `/metrics` answers in the OpenMetrics text format, to a read token of scope
//! `metrics` - an administrator's. Two kinds of figure, kept apart on purpose:
//!
//! * **What happened in this process** - requests and how long they took, the
//!   runs of the background tasks. Counted here as they happen and gone with
//!   the process: Prometheus reads a restart as a counter reset, as it should.
//! * **What the database holds** - the webhook queue. Read from the rows at
//!   each scrape, never mirrored in memory: a count kept in two places is two
//!   counts, and only one of them survives a restart.
//!
//! No figure names a person. A request is counted by the pattern of its route
//! (`/api/v1/users/{id}/days`), never by the path it took, and a webhook
//! destination by the label the operator gave it.
//!
//! The names and labels are a contract, like the BI API's columns: an alert
//! written against a renamed series does not fail - it goes quiet, which reads
//! as "all is well". Within this major version a figure is only ever added.

pub mod process;

use std::{
    fmt::Write as _,
    sync::{Arc, atomic::AtomicU64},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{MatchedPath, Request, State},
    http::{Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue, LabelValueEncoder, text},
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::{Registry, Unit},
};
use sqlx::PgPool;

use crate::{
    app::AppState,
    read_token::{Metrics, ReadToken},
    webhooks::{self, Webhooks},
};

/// The media type of the OpenMetrics text format. Prometheus asks for it by
/// name; it also carries what the older text format has no room for - the
/// units, and the line that says the scrape arrived whole.
pub const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// The edges of the request-duration buckets, in seconds: Prometheus's own
/// defaults, which a stock latency panel already expects.
const DURATION_BUCKETS: [f64; 11] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

/// How long a scrape waits on the database for the figures it holds.
///
/// Prometheus gives up on a scrape after ten seconds by default, and a scrape
/// it gave up on loses every figure - the process's included, which are the
/// ones that say why the database is slow. Past this, the scrape answers
/// without the database's figures instead.
const DATABASE_WAIT: Duration = Duration::from_secs(2);

/// A task the server runs on its own, on a timer.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Task {
    /// The pass that raises and clears alerts (ADR 0018).
    AlertSweep,
    /// The pass that sends what the webhook queue holds (ADR 0019).
    WebhookDispatch,
}

impl Task {
    pub const ALL: [Task; 2] = [Task::AlertSweep, Task::WebhookDispatch];

    pub fn name(self) -> &'static str {
        match self {
            Task::AlertSweep => "alert_sweep",
            Task::WebhookDispatch => "webhook_dispatch",
        }
    }
}

impl EncodeLabelValue for Task {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
        encoder.write_str(self.name())
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct BuildLabels {
    version: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RequestLabels {
    method: &'static str,
    route: String,
    status: u16,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RouteLabels {
    method: &'static str,
    route: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RunLabels {
    task: Task,
    outcome: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct TaskLabels {
    task: Task,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ConnectionLabels {
    state: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DeliveryLabels {
    destination: String,
    state: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DestinationLabels {
    destination: String,
}

type Seconds = Gauge<f64, AtomicU64>;
type Durations = Family<RouteLabels, Histogram, fn() -> Histogram>;

/// What this process has counted since it started.
///
/// One per process, shared by the router and the background tasks. Not a
/// global: a test builds its own and reads back exactly what it caused.
pub struct Recorder {
    registry: Registry,
    requests: Family<RequestLabels, Counter>,
    durations: Durations,
    runs: Family<RunLabels, Counter>,
    last_success: Family<TaskLabels, Seconds>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        let started = unix_seconds(SystemTime::now());

        // The version as a label on a constant 1, the shape every exporter
        // uses: a panel joins on it, and an upgrade shows as the label
        // changing at the moment it happened.
        let build: Family<BuildLabels, Gauge> = Family::default();
        build
            .get_or_create(&BuildLabels {
                version: env!("CARGO_PKG_VERSION"),
            })
            .set(1);
        registry.register("kasl_build_info", "The version of kasl-server answering; always 1", build);

        // When this process began serving. The name every client uses, so a
        // stock "uptime" panel reads it.
        let start: Seconds = Gauge::default();
        start.set(started);
        registry.register_with_unit(
            "process_start_time",
            "When the process started, in seconds since the Unix epoch",
            Unit::Seconds,
            start,
        );

        let requests: Family<RequestLabels, Counter> = Family::default();
        registry.register("kasl_http_requests", "Requests answered, by method, route pattern and status", requests.clone());

        let durations: Durations = Family::new_with_constructor(|| Histogram::new(DURATION_BUCKETS));
        registry.register_with_unit(
            "kasl_http_request_duration",
            "Time from a request arriving to its response's head, by method and route pattern",
            Unit::Seconds,
            durations.clone(),
        );

        let runs: Family<RunLabels, Counter> = Family::default();
        registry.register("kasl_background_runs", "Runs of the server's own timed tasks, by how they ended", runs.clone());

        let last_success: Family<TaskLabels, Seconds> = Family::default();
        registry.register_with_unit(
            "kasl_background_last_success_timestamp",
            "When each timed task last completed; the process's start until it first has",
            Unit::Seconds,
            last_success.clone(),
        );

        for task in Task::ALL {
            // Every series a rule may ask about exists from the first scrape,
            // at zero: a counter that appears only on its first error makes
            // `increase()` miss that error.
            for outcome in ["ok", "error"] {
                let _ = runs.get_or_create(&RunLabels { task, outcome });
            }
            // The start, not nothing: a task stuck in its very first run never
            // completes, and a series that is absent cannot grow old enough to
            // alert on.
            last_success.get_or_create(&TaskLabels { task }).set(started);
        }

        Self {
            registry,
            requests,
            durations,
            runs,
            last_success,
        }
    }

    /// Counts one answered request.
    pub fn observe_request(&self, method: &Method, route: String, status: u16, elapsed: Duration) {
        let method = method_label(method);
        self.durations
            .get_or_create(&RouteLabels { method, route: route.clone() })
            .observe(elapsed.as_secs_f64());
        self.requests.get_or_create(&RequestLabels { method, route, status }).inc();
    }

    /// Counts one run of a timed task, and when it went well, the moment.
    pub fn record_run(&self, task: Task, succeeded: bool) {
        let outcome = if succeeded { "ok" } else { "error" };
        self.runs.get_or_create(&RunLabels { task, outcome }).inc();
        if succeeded {
            self.last_success.get_or_create(&TaskLabels { task }).set(unix_seconds(SystemTime::now()));
        }
    }

    /// Everything, as one OpenMetrics document: what this process counted,
    /// then what the process and the database say right now.
    pub async fn render(&self, pool: &PgPool, webhooks: &Webhooks) -> String {
        let now = scrape(pool, webhooks).await;
        let mut text = String::new();
        text::encode_registry(&mut text, &self.registry)
            .and_then(|()| text::encode_registry(&mut text, &now))
            .and_then(|()| text::encode_eof(&mut text))
            .expect("writing into a String cannot fail");
        text
    }
}

/// The figures read at the moment of a scrape, in a registry of their own -
/// built for this scrape and dropped after it, so a destination removed from
/// the configuration leaves no series behind.
async fn scrape(pool: &PgPool, webhooks: &Webhooks) -> Registry {
    let mut registry = Registry::default();

    if let Some(figures) = process::read() {
        let cpu: Counter<f64, AtomicU64> = Counter::default();
        cpu.inc_by(figures.cpu_seconds);
        registry.register_with_unit("process_cpu", "User and system CPU time spent", Unit::Seconds, cpu);
        registry.register_with_unit("process_resident_memory", "Resident memory size", Unit::Bytes, gauge(figures.resident_bytes));
        registry.register_with_unit("process_virtual_memory", "Virtual memory size", Unit::Bytes, gauge(figures.virtual_bytes));
        registry.register("process_threads", "Threads in the process", gauge(figures.threads));
        registry.register("process_open_fds", "Open file descriptors", gauge(figures.open_fds));
        if let Some(max) = figures.max_fds {
            registry.register("process_max_fds", "The limit on open file descriptors", gauge(max));
        }
    }

    // The pool as it stands. Busy against the limit is the figure that says
    // requests are queueing for a connection rather than for the database.
    let open = pool.size();
    let idle = u32::try_from(pool.num_idle()).unwrap_or(u32::MAX).min(open);
    let connections: Family<ConnectionLabels, Gauge> = Family::default();
    connections.get_or_create(&ConnectionLabels { state: "idle" }).set(idle.into());
    connections.get_or_create(&ConnectionLabels { state: "busy" }).set((open - idle).into());
    registry.register(
        "kasl_database_connections",
        "Open connections to the database, by whether a request holds them",
        connections,
    );
    registry.register(
        "kasl_database_connections_max",
        "The most connections the server opens to the database",
        gauge(u64::from(pool.options().get_max_connections())),
    );

    match tokio::time::timeout(DATABASE_WAIT, from_the_database(pool, webhooks)).await {
        Ok(Ok(held)) => {
            let round_trip: Seconds = Gauge::default();
            round_trip.set(held.round_trip.as_secs_f64());
            registry.register_with_unit(
                "kasl_database_round_trip",
                "How long the database took to answer a trivial query",
                Unit::Seconds,
                round_trip,
            );

            let deliveries: Family<DeliveryLabels, Gauge> = Family::default();
            let oldest: Family<DestinationLabels, Seconds> = Family::default();
            for queue in held.queues {
                for (state, count) in [("pending", queue.pending), ("delivered", queue.delivered), ("abandoned", queue.abandoned)] {
                    deliveries
                        .get_or_create(&DeliveryLabels {
                            destination: queue.destination.clone(),
                            state,
                        })
                        .set(count);
                }
                let waited = queue.oldest_pending_at.map_or(0.0, |queued| (Utc::now() - queued).as_seconds_f64().max(0.0));
                oldest
                    .get_or_create(&DestinationLabels {
                        destination: queue.destination,
                    })
                    .set(waited);
            }
            registry.register(
                "kasl_webhook_deliveries",
                "Deliveries to each configured destination, by where they stand",
                deliveries,
            );
            registry.register_with_unit(
                "kasl_webhook_oldest_pending_age",
                "How long the oldest delivery still waiting for each destination has waited; 0 when none waits",
                Unit::Seconds,
                oldest,
            );
        }
        Ok(Err(error)) => tracing::warn!(%error, "a scrape could not read the database's figures"),
        Err(_) => tracing::warn!(waited = ?DATABASE_WAIT, "a scrape gave up waiting for the database's figures"),
    }

    registry
}

struct Held {
    round_trip: Duration,
    queues: Vec<webhooks::Queue>,
}

async fn from_the_database(pool: &PgPool, webhooks: &Webhooks) -> Result<Held, sqlx::Error> {
    let started = Instant::now();
    sqlx::query("SELECT 1").execute(pool).await?;
    let round_trip = started.elapsed();
    let queues = webhooks::queues(pool, webhooks).await?;
    Ok(Held { round_trip, queues })
}

fn gauge(value: u64) -> Gauge {
    let gauge = Gauge::default();
    gauge.set(i64::try_from(value).unwrap_or(i64::MAX));
    gauge
}

fn unix_seconds(at: SystemTime) -> f64 {
    at.duration_since(UNIX_EPOCH).map_or(0.0, |since| since.as_secs_f64())
}

/// A method as a label. Anything outside the standard set is one value: the
/// method is the client's to choose, and a label that takes whatever arrives
/// is a series per made-up word.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        _ => "OTHER",
    }
}

/// What a request is counted under: the pattern of the route that answered,
/// or one of two words for a request no route claimed.
///
/// Never the path itself. A path carries ids - a series per person - and
/// anything a scanner cares to ask for.
fn route_label(matched: Option<&str>, path: &str) -> String {
    match matched {
        Some(matched) => matched.to_owned(),
        // `/api` answers an unknown endpoint with a JSON 404; everywhere else
        // the web UI answers (ADR 0012).
        None if path == "/api" || path.starts_with("/api/") => "unmatched".to_owned(),
        None => "web".to_owned(),
    }
}

/// The middleware every request passes through on its way to a route.
pub async fn track(State(recorder): State<Arc<Recorder>>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let route = route_label(request.extensions().get::<MatchedPath>().map(MatchedPath::as_str), request.uri().path());
    let response = next.run(request).await;
    recorder.observe_request(&method, route, response.status().as_u16(), started.elapsed());
    response
}

/// `GET /metrics` - for an administrator's read token of scope `metrics`.
pub async fn serve(State(state): State<AppState>, _token: ReadToken<Metrics>) -> Response {
    let body = state.recorder.render(&state.pool, &state.webhooks).await;
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_counted_by_its_route_pattern() {
        assert_eq!(
            route_label(Some("/api/v1/users/{id}/days"), "/api/v1/users/0f3c/days"),
            "/api/v1/users/{id}/days"
        );
    }

    #[test]
    fn a_request_no_route_claimed_is_one_of_two_words() {
        assert_eq!(route_label(None, "/api/v1/wp-login.php"), "unmatched");
        assert_eq!(route_label(None, "/api"), "unmatched");
        assert_eq!(route_label(None, "/team/0f3c"), "web");
        assert_eq!(route_label(None, "/apiary"), "web", "a page that only starts with the letters is the web UI's");
    }

    #[test]
    fn a_made_up_method_is_one_label() {
        assert_eq!(method_label(&Method::GET), "GET");
        assert_eq!(method_label(&Method::from_bytes(b"BREW").unwrap()), "OTHER");
    }
}
