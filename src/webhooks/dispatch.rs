//! Sending what is queued: in order per destination, retried, and given up on
//! in writing.
//!
//! The loop wakes every few seconds and, for each destination, sends the
//! oldest thing still in flight - and then the next, while they keep
//! succeeding. A failure stops that destination until its retry is due, which
//! is what keeps "resolved" from arriving before "raised": later events wait
//! behind the one that has not got through. Other destinations are not held
//! up by it.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::{StatusCode, header};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    Destination, Event, Kind, Webhooks,
    destination::Target,
    render::{self, Markup},
    sign,
};
use crate::error::ApiError;

/// How often the dispatcher looks for work.
///
/// Seconds, not the sweep's minutes: an alert is raised at most every five
/// minutes, but an acknowledgement or a test is somebody at a screen waiting
/// to see the channel light up.
const TICK: Duration = Duration::from_secs(5);

/// How long one request may take before it counts as failed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The most one destination is sent in one tick. A backlog after an outage
/// drains over a few ticks rather than holding the loop for minutes.
const BURST: usize = 50;

/// The waits between attempts. Nine tries across about a day: long enough to
/// ride out a chat service's bad night, short enough that an alert is not
/// delivered on Thursday about Monday.
const RETRY_AFTER: [TimeDelta; 8] = [
    TimeDelta::seconds(30),
    TimeDelta::minutes(2),
    TimeDelta::minutes(10),
    TimeDelta::minutes(30),
    TimeDelta::hours(1),
    TimeDelta::hours(3),
    TimeDelta::hours(6),
    TimeDelta::hours(12),
];

/// The longest a receiver's `Retry-After` is taken at its word.
const LONGEST_REQUESTED_WAIT: TimeDelta = TimeDelta::hours(1);

/// How much of a refusal's body is kept. Enough for Slack's `no_service` or
/// Telegram's `chat not found`; not enough to store a page of HTML.
const ERROR_BODY: usize = 300;

/// What one tick did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Dispatched {
    pub delivered: u64,
    /// Failed, and scheduled to be tried again.
    pub retrying: u64,
    /// Failed for good: refused, out of retries, or sent to a destination no
    /// longer configured.
    pub abandoned: u64,
}

/// The HTTP client every delivery goes through.
///
/// TLS from rustls with the `ring` provider and the Mozilla roots compiled in,
/// the same stack the database connection already uses, so a delivery does
/// not depend on which certificates the host happens to have installed.
pub fn client() -> reqwest::Client {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();

    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("kasl-server/", env!("CARGO_PKG_VERSION")))
        // A hook that redirects is a hook that moved, and following it would
        // send the body somewhere nobody configured.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("the HTTP client builds from a fixed configuration")
}

/// One queued delivery, as the dispatcher reads it.
#[derive(Debug, sqlx::FromRow)]
struct Pending {
    id: Uuid,
    attempts: i32,
    next_attempt_at: DateTime<Utc>,
    payload: sqlx::types::Json<Event>,
}

/// What came of one attempt.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Delivered,
    /// Worth trying again: the receiver was down, slow, or asked us to wait.
    Retry {
        status: Option<u16>,
        error: String,
        wait: Option<TimeDelta>,
    },
    /// Not worth trying again: the receiver refused this request for a
    /// reason sending it again will not change.
    Refused {
        status: u16,
        error: String,
    },
}

/// Sends everything that is due, once.
pub async fn dispatch_due(pool: &PgPool, webhooks: &Webhooks, client: &reqwest::Client, now: DateTime<Utc>) -> Result<Dispatched, ApiError> {
    let mut dispatched = Dispatched::default();

    let waiting: Vec<String> = sqlx::query_scalar("SELECT DISTINCT destination FROM webhook_deliveries WHERE delivered_at IS NULL AND abandoned_at IS NULL")
        .fetch_all(pool)
        .await?;

    for name in waiting {
        let Some(destination) = webhooks.get(&name) else {
            // Its variable was removed. Given up on in writing rather than
            // left pending forever, where it would read on the screen as
            // "still trying".
            let dropped = sqlx::query(
                "UPDATE webhook_deliveries SET abandoned_at = $2, last_error = $3
                 WHERE destination = $1 AND delivered_at IS NULL AND abandoned_at IS NULL",
            )
            .bind(&name)
            .bind(now)
            .bind(format!("no destination named `{name}` is configured any more"))
            .execute(pool)
            .await?;
            dispatched.abandoned += dropped.rows_affected();
            continue;
        };

        for _ in 0..BURST {
            let head: Option<Pending> = sqlx::query_as(
                "SELECT id, attempts, next_attempt_at, payload FROM webhook_deliveries
                 WHERE destination = $1 AND delivered_at IS NULL AND abandoned_at IS NULL
                 ORDER BY created_at, id
                 LIMIT 1",
            )
            .bind(&name)
            .fetch_optional(pool)
            .await?;

            // Nothing left, or the oldest is waiting for its retry - in which
            // case everything behind it waits too.
            let Some(head) = head.filter(|head| head.next_attempt_at <= now) else { break };

            let outcome = send(client, destination, &head.payload, now).await;
            let attempts = head.attempts + 1;
            match outcome {
                Outcome::Delivered => {
                    sqlx::query("UPDATE webhook_deliveries SET delivered_at = $2, attempts = $3, last_status = NULL, last_error = NULL WHERE id = $1")
                        .bind(head.id)
                        .bind(now)
                        .bind(attempts)
                        .execute(pool)
                        .await?;
                    dispatched.delivered += 1;
                    continue;
                }
                Outcome::Retry { status, error, wait } => {
                    let error = destination.redact(&error);
                    match RETRY_AFTER.get(head.attempts as usize) {
                        Some(scheduled) => {
                            let wait = wait.map_or(*scheduled, |asked| asked.clamp(*scheduled, LONGEST_REQUESTED_WAIT.max(*scheduled)));
                            sqlx::query("UPDATE webhook_deliveries SET attempts = $2, next_attempt_at = $3, last_status = $4, last_error = $5 WHERE id = $1")
                                .bind(head.id)
                                .bind(attempts)
                                .bind(now + wait)
                                .bind(status.map(i32::from))
                                .bind(&error)
                                .execute(pool)
                                .await?;
                            tracing::warn!(destination = %name, attempts, %error, "a webhook delivery failed; it will be tried again");
                            dispatched.retrying += 1;
                        }
                        None => {
                            abandon(pool, head.id, attempts, now, status, &format!("gave up after {attempts} attempts: {error}")).await?;
                            tracing::warn!(destination = %name, attempts, %error, "gave up on a webhook delivery");
                            dispatched.abandoned += 1;
                        }
                    }
                }
                Outcome::Refused { status, error } => {
                    let error = destination.redact(&error);
                    abandon(pool, head.id, attempts, now, Some(status), &error).await?;
                    tracing::warn!(destination = %name, status, %error, "a webhook receiver refused a delivery; not trying again");
                    dispatched.abandoned += 1;
                }
            }
            break;
        }
    }

    Ok(dispatched)
}

async fn abandon(pool: &PgPool, id: Uuid, attempts: i32, now: DateTime<Utc>, status: Option<u16>, error: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE webhook_deliveries SET abandoned_at = $2, attempts = $3, last_status = $4, last_error = $5 WHERE id = $1")
        .bind(id)
        .bind(now)
        .bind(attempts)
        .bind(status.map(i32::from))
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

/// Makes one attempt.
async fn send(client: &reqwest::Client, destination: &Destination, event: &Event, now: DateTime<Utc>) -> Outcome {
    let request = match (&destination.target, destination.kind) {
        (Target::Hook { url }, Kind::Slack) => client.post(url).json(&render::hook_body(render::text(event, destination, Markup::Slack))),
        (Target::Hook { url }, _) => client.post(url).json(&render::hook_body(render::text(event, destination, Markup::Markdown))),
        (Target::Telegram { token, chat, api }, _) => client
            .post(format!("{api}/bot{token}/sendMessage"))
            .json(&render::telegram_body(chat, render::text(event, destination, Markup::TelegramHtml))),
        (Target::Json { url, secret }, _) => {
            let body = match serde_json::to_vec(event) {
                Ok(body) => body,
                Err(error) => {
                    return Outcome::Refused {
                        status: 0,
                        error: format!("the event could not be written as JSON: {error}"),
                    };
                }
            };
            client
                .post(url)
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-Kasl-Event", event.event.name())
                .header("X-Kasl-Event-Id", event.id.to_string())
                .header("X-Kasl-Signature", sign::signature(secret, now.timestamp(), &body))
                .body(body)
        }
    };

    let response = match request.send().await {
        Ok(response) => response,
        // `without_url`: reqwest's error names the address it was sending to,
        // and the address is the credential.
        Err(error) => {
            return Outcome::Retry {
                status: None,
                error: describe(&error.without_url()),
                wait: None,
            };
        }
    };

    let status = response.status();
    if status.is_success() {
        return Outcome::Delivered;
    }

    let wait = retry_after(&response);
    let body: String = response.text().await.unwrap_or_default().chars().take(ERROR_BODY).collect();
    let error = if body.trim().is_empty() {
        format!("the receiver answered {status}")
    } else {
        format!("the receiver answered {status}: {}", body.trim())
    };

    classify(status, error, wait)
}

/// Which failures are worth another try.
///
/// A 4xx says the request itself is wrong - a hook that was deleted, a bot
/// removed from the chat, a token revoked - and the same request will be
/// wrong in an hour too. The exceptions are the two that say "not now":
/// 408 and 429. Everything else, including every 5xx, is the receiver's bad
/// moment and gets retried.
fn classify(status: StatusCode, error: String, wait: Option<TimeDelta>) -> Outcome {
    let code = status.as_u16();
    if status.is_client_error() && code != 408 && code != 429 {
        Outcome::Refused { status: code, error }
    } else {
        Outcome::Retry {
            status: Some(code),
            error,
            wait,
        }
    }
}

/// A `Retry-After` given in seconds. The HTTP-date form is not honoured: no
/// chat service this sends to uses it, and a wrongly parsed date is a wait of
/// years.
fn retry_after(response: &reqwest::Response) -> Option<TimeDelta> {
    let seconds: i64 = response.headers().get(header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    (seconds >= 0).then(|| TimeDelta::seconds(seconds))
}

/// An error and its causes in one line: "error sending request: connection
/// refused", where the top level alone says only the first half.
fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

/// Runs the dispatcher for as long as the server runs.
///
/// A tick that fails is logged and the loop goes on: the queue is in the
/// database, so nothing is lost by a tick that did not run, and a database
/// blip must not leave deliveries frozen until a restart.
pub fn run_dispatcher(pool: PgPool, webhooks: Arc<Webhooks>) {
    tokio::spawn(async move {
        let client = client();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match dispatch_due(&pool, &webhooks, &client, Utc::now()).await {
                Ok(done) if done != Dispatched::default() => {
                    tracing::info!(
                        delivered = done.delivered,
                        retrying = done.retrying,
                        abandoned = done.abandoned,
                        "dispatched webhooks"
                    );
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "a webhook dispatch failed; the next one picks up where it left off"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_final_and_a_bad_moment_is_not() {
        for code in [400, 401, 403, 404, 410] {
            assert!(
                matches!(classify(StatusCode::from_u16(code).unwrap(), String::new(), None), Outcome::Refused { .. }),
                "{code} will not change by being asked again"
            );
        }
        for code in [408, 429, 500, 502, 503, 504] {
            assert!(
                matches!(classify(StatusCode::from_u16(code).unwrap(), String::new(), None), Outcome::Retry { .. }),
                "{code} is worth another try"
            );
        }
    }

    #[test]
    fn the_retries_span_about_a_day() {
        let total: TimeDelta = RETRY_AFTER.iter().copied().sum();
        assert!(total > TimeDelta::hours(20) && total < TimeDelta::hours(26), "{total}");
        assert!(RETRY_AFTER.windows(2).all(|pair| pair[0] < pair[1]), "each wait longer than the last");
    }
}
