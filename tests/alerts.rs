//! Alerts, against a live database.
//!
//! The rules themselves are unit-tested in the module, where an observation is
//! a struct rather than a join. What is only reachable here is the half that
//! makes an alert a different object from a signal: that the sweep
//! **reconciles** instead of inserting, so a fortnight of silence is one row
//! and not one per sweep; that an alert closes itself when its condition stops
//! being true; that an answered one stays answered; and that the feed applies
//! the same visibility rule as every other route about other people.
//!
//! The sweep is driven directly with a fixed `now` rather than by waiting for
//! the background timer. A test that slept for five minutes would be a test
//! nobody runs.

mod support;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{Value, json};
use support::TestServer;
use uuid::Uuid;

/// A finished day running `hours` from nine in the morning.
fn day(date: NaiveDate, hours: i64) -> Value {
    json!({
        "date": date.to_string(),
        "started_at": format!("{date}T09:00:00+00:00"),
        "ended_at": format!("{date}T{:02}:00:00+00:00", 9 + hours),
        "pauses": [],
        "tasks": []
    })
}

/// A weekday close enough to now to be inside the sweep's window.
fn a_recent_weekday() -> NaiveDate {
    let mut date = Utc::now().date_naive() - Duration::days(1);
    while calendar_weekend(date) {
        date -= Duration::days(1);
    }
    date
}

fn calendar_weekend(date: NaiveDate) -> bool {
    use chrono::Datelike;
    matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
}

/// The id of a seeded account.
async fn user_id(server: &TestServer, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .fetch_one(&server.pool)
        .await
        .expect("the fixture account should exist")
}

/// Moves an agent's last contact back, which is the only way to make somebody
/// silent inside a test: provisioning stamps it as now.
async fn last_seen(server: &TestServer, email: &str, at: DateTime<Utc>) {
    sqlx::query("UPDATE agents SET last_seen_at = $2 WHERE user_id = (SELECT id FROM users WHERE lower(email) = lower($1))")
        .bind(email)
        .bind(at)
        .execute(&server.pool)
        .await
        .expect("the fixture should apply");
}

/// The open alerts in the database, by rule, regardless of who may see them.
async fn open_rules(server: &TestServer) -> Vec<String> {
    sqlx::query_scalar("SELECT rule::text FROM alerts WHERE state = 'open' ORDER BY rule::text")
        .fetch_all(&server.pool)
        .await
        .expect("the alerts should be readable")
}

async fn sweep(server: &TestServer, now: DateTime<Utc>) -> kasl_server::alerts::Swept {
    kasl_server::alerts::sweep(&server.pool, now, &kasl_server::webhooks::Webhooks::default())
        .await
        .expect("the sweep should run")
}

#[tokio::test]
async fn a_silent_agent_becomes_one_alert_and_stays_one() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;

    let first = sweep(&server, now).await;
    assert_eq!(first.raised, 1, "thirty hours of silence is past the twelve-hour default");
    assert_eq!(open_rules(&server).await, vec!["no_agent_data"]);

    // The point of reconciling rather than inserting. Four more sweeps over
    // the same unchanged silence must not produce four more rows: a manager
    // opening the dashboard after a quiet weekend would otherwise find the
    // same fact about the same person listed dozens of times, which is how a
    // feed becomes something nobody reads.
    for _ in 0..4 {
        let again = sweep(&server, now + Duration::minutes(5)).await;
        assert_eq!(again.raised, 0, "the condition has not changed, so nothing new happened");
        assert_eq!(again.unchanged, 1);
    }
    assert_eq!(open_rules(&server).await, vec!["no_agent_data"]);

    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM alerts").fetch_one(&server.pool).await.unwrap();
    assert_eq!(total, 1, "five sweeps over one silence is one row");

    server.close().await;
}

#[tokio::test]
async fn an_alert_that_stops_being_true_closes_itself() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;
    assert_eq!(open_rules(&server).await, vec!["no_agent_data"]);

    // The agent comes back. Nobody clicks anything - the whole reason the
    // sweep reconciles is that most alerts end this way, and a feed that
    // needed a human to close each one would fill up with history.
    last_seen(&server, "employee@example.test", now).await;
    let swept = sweep(&server, now + Duration::minutes(5)).await;

    assert_eq!(swept.resolved, 1);
    assert!(open_rules(&server).await.is_empty(), "the condition is gone, so the alert is");

    // Resolved, not deleted, and with the moment it ended. "How much of what
    // the server shouted about was real" is the one question worth asking of
    // this table, and a row that vanishes cannot answer it.
    let (state, resolved_at): (String, Option<DateTime<Utc>>) = sqlx::query_as("SELECT state::text, resolved_at FROM alerts")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(state, "resolved");
    assert!(resolved_at.is_some(), "a resolved alert knows when it ended");

    server.close().await;
}

#[tokio::test]
async fn an_acknowledged_alert_is_not_raised_again_while_it_holds() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    server.add_admin("boss@example.test", "supersecret").await;
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    let (_, cookie, _) = server.login("boss@example.test", "supersecret").await;
    let (status, feed) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);
    let id = feed["alerts"][0]["id"].as_str().expect("the feed should carry the alert").to_string();

    let (status, _, _) = server
        .post_with_cookie(&format!("/api/v1/alerts/{id}/acknowledge"), cookie.as_deref(), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    // The condition is still perfectly true - the agent is still silent - and
    // that is exactly the case. A server that re-raised it on the next sweep
    // would make the button meaningless, which is the behaviour that teaches
    // people to ignore alerts altogether.
    let swept = sweep(&server, now + Duration::minutes(5)).await;
    assert_eq!(swept.raised, 0, "a person answered this; the server does not argue");
    assert!(open_rules(&server).await.is_empty());

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM alerts").fetch_one(&server.pool).await.unwrap();
    assert_eq!(count, 1);

    // And it says who answered it, by name, so the feed does not need a
    // second request to tell "somebody decided it was fine" from "it went
    // away on its own".
    let (_, feed) = server.get_with_cookie("/api/v1/alerts?state=acknowledged", cookie.as_deref()).await;
    assert_eq!(feed["alerts"][0]["acknowledged_by"], "boss@example.test");
    assert_eq!(feed["alerts"][0]["state"], "acknowledged");

    server.close().await;
}

#[tokio::test]
async fn an_acknowledgement_lasts_exactly_as_long_as_its_condition() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    server.add_admin("boss@example.test", "supersecret").await;
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    let (_, cookie, _) = server.login("boss@example.test", "supersecret").await;
    let (_, feed) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    let id = feed["alerts"][0]["id"].as_str().unwrap().to_string();
    let (status, _, _) = server
        .post_with_cookie(&format!("/api/v1/alerts/{id}/acknowledge"), cookie.as_deref(), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    // The agent comes back, and some days later goes quiet again. This must
    // be an alert: the dismissal answered *that* silence, not every silence
    // this person will ever have. A suppression that never expired would mean
    // one click permanently muted a rule for one employee, which nothing on
    // any screen would ever say.
    last_seen(&server, "employee@example.test", now).await;
    sweep(&server, now + Duration::minutes(5)).await;

    let later = now + Duration::days(10);
    last_seen(&server, "employee@example.test", later - Duration::hours(30)).await;
    let swept = sweep(&server, later).await;

    assert_eq!(swept.raised, 1, "a new silence is a new alert, dismissal or not");
    assert_eq!(open_rules(&server).await, vec!["no_agent_data"]);

    // And the answered one still says it was answered - the record of a person
    // having looked is not overwritten by the condition ending.
    let states: Vec<String> = sqlx::query_scalar("SELECT state::text FROM alerts ORDER BY fired_at")
        .fetch_all(&server.pool)
        .await
        .unwrap();
    assert_eq!(states, vec!["acknowledged", "open"]);

    server.close().await;
}

#[tokio::test]
async fn the_same_condition_returning_later_is_a_new_alert() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    last_seen(&server, "employee@example.test", now).await;
    sweep(&server, now + Duration::minutes(5)).await;
    assert!(open_rules(&server).await.is_empty());

    // Quiet again a week later. This is a genuinely new event, and the
    // partial unique index is on the open rows alone precisely so that the
    // resolved March one does not block the July one.
    let later = now + Duration::days(7);
    last_seen(&server, "employee@example.test", later - Duration::hours(30)).await;
    let swept = sweep(&server, later).await;

    assert_eq!(swept.raised, 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM alerts").fetch_one(&server.pool).await.unwrap();
    assert_eq!(count, 2, "two separate silences are two rows");

    server.close().await;
}

#[tokio::test]
async fn overwork_is_raised_from_real_days_and_carries_the_norm() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    let date = a_recent_weekday();
    // Thirteen hours against the default eight-hour norm: past the 1.5 factor.
    let (status, _) = server.post_day(&server.token, day(date, 13)).await;
    assert_eq!(status, StatusCode::OK);
    // Not silent: the upload just stamped the agent, and this test is about
    // the other rule.
    last_seen(&server, "employee@example.test", now).await;

    let swept = sweep(&server, now).await;
    assert_eq!(swept.raised, 1);
    assert_eq!(open_rules(&server).await, vec!["overwork"]);

    let (observed, against, subject): (i64, Option<i64>, Option<NaiveDate>) =
        sqlx::query_as("SELECT observed_seconds, against_seconds, subject_date FROM alerts WHERE rule = 'overwork'")
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert_eq!(observed, 13 * 3600, "the hours actually worked, from the uploaded day");
    assert_eq!(against, Some(8 * 3600), "the norm travels with the alert, not a percentage");
    assert_eq!(subject, Some(date));

    server.close().await;
}

#[tokio::test]
async fn an_alerts_figures_do_not_drift_after_it_fires() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(13)).await;
    sweep(&server, now).await;

    let first: i64 = sqlx::query_scalar("SELECT observed_seconds FROM alerts WHERE state = 'open'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(first, 13 * 3600);

    // A fortnight later the same silence is 349 hours long, and the alert must
    // still read what it read when it fired. Rewriting it would make a
    // sentence a manager already read change under them, and "quiet since
    // Monday, 13 h" is the statement that was true at the moment somebody was
    // told - which is the whole point of storing an observation rather than
    // recomputing one.
    sweep(&server, now + Duration::days(14)).await;
    let later: i64 = sqlx::query_scalar("SELECT observed_seconds FROM alerts WHERE state = 'open'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(later, first, "an alert says what it said when it fired");

    server.close().await;
}

#[tokio::test]
async fn a_pulse_counts_as_the_agent_speaking() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    // The token has not been used in a day and a half - but the machine has
    // been pulsing all along. This is the ordinary state of an agent whose
    // employee is on holiday: it says `idle` every minute and uploads nothing.
    //
    // The first version of this rule read `last_seen_at` alone, and a live run
    // against the demo flagged nine of twelve people as silent, six of them
    // having pulsed seconds earlier. The two stamps answer different questions
    // (ADR 0014) and a rule about silence has to read both.
    last_seen(&server, "employee@example.test", now - Duration::hours(36)).await;
    sqlx::query(
        "UPDATE agents SET heartbeat_state = 'idle', heartbeat_at = $1, heartbeat_received_at = $1
         WHERE user_id = (SELECT id FROM users WHERE lower(email) = 'employee@example.test')",
    )
    .bind(now - Duration::minutes(2))
    .execute(&server.pool)
    .await
    .expect("the pulse fixture should apply");

    let swept = sweep(&server, now).await;
    assert_eq!(swept.raised, 0, "a machine that pulsed two minutes ago is not silent");
    assert!(open_rules(&server).await.is_empty());

    // And when the pulse goes stale too, the silence is real.
    sqlx::query("UPDATE agents SET heartbeat_received_at = $1 WHERE heartbeat_received_at IS NOT NULL")
        .bind(now - Duration::hours(20))
        .execute(&server.pool)
        .await
        .unwrap();
    let swept = sweep(&server, now).await;
    assert_eq!(swept.raised, 1, "both stamps stale is genuine silence");

    // Measured from the freshest of the two, not from the older one.
    let observed: i64 = sqlx::query_scalar("SELECT observed_seconds FROM alerts WHERE state = 'open'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert!((observed - 20 * 3600).abs() <= 2, "expected about twenty hours, got {observed}");

    server.close().await;
}

#[tokio::test]
async fn somebody_with_no_agent_raises_nothing() {
    let Some(server) = TestServer::start().await else { return };

    // An account with no token at all: the case an installation is in on its
    // first afternoon, halfway through handing them out. Alerting about
    // everybody then is how a feature gets switched off on day one.
    sqlx::query("INSERT INTO users (email, display_name, role) VALUES ('unequipped@example.test', 'Unequipped', 'employee')")
        .execute(&server.pool)
        .await
        .expect("the fixture account should insert");

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now).await;

    let swept = sweep(&server, now).await;
    assert_eq!(swept.raised, 0, "nothing was ever asked of that account");
    assert!(open_rules(&server).await.is_empty());

    server.close().await;
}

#[tokio::test]
async fn the_feed_shows_only_people_the_reader_may_see() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    server.add_admin("root@example.test", "supersecret").await;
    server.add_agent("other@example.test", "other-token").await;
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    last_seen(&server, "other@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;
    assert_eq!(open_rules(&server).await.len(), 2, "both are silent");

    // A manager of nobody: they have the role, and no department, so the
    // visibility rule gives them exactly themselves. The rule is the same
    // constant every other route about other people uses; this is the test
    // that it was not forgotten on a third pair of routes.
    sqlx::query("UPDATE users SET role = 'manager' WHERE lower(email) = 'other@example.test'")
        .execute(&server.pool)
        .await
        .unwrap();
    server.set_password("other@example.test", "supersecret").await;

    let (_, cookie, _) = server.login("other@example.test", "supersecret").await;
    let (status, feed) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);

    let alerts = feed["alerts"].as_array().expect("the feed is a list");
    assert_eq!(alerts.len(), 1, "a manager of nobody sees only their own: {alerts:?}");
    assert_eq!(alerts[0]["user_id"], user_id(&server, "other@example.test").await.to_string());
    assert_eq!(feed["open"], 1, "the badge counts what this reader may see, not the table");

    // And the administrator sees both.
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;
    let (_, feed) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    assert_eq!(feed["alerts"].as_array().unwrap().len(), 2);

    server.close().await;
}

#[tokio::test]
async fn answering_someone_elses_alert_is_a_404_not_a_403() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    server.add_agent("stranger@example.test", "stranger-token").await;
    last_seen(&server, "stranger@example.test", now - Duration::hours(30)).await;
    last_seen(&server, "employee@example.test", now).await;
    sweep(&server, now).await;

    let id: Uuid = sqlx::query_scalar("SELECT id FROM alerts WHERE state = 'open'")
        .fetch_one(&server.pool)
        .await
        .unwrap();

    sqlx::query("UPDATE users SET role = 'manager' WHERE lower(email) = 'employee@example.test'")
        .execute(&server.pool)
        .await
        .unwrap();
    server.set_password("employee@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("employee@example.test", "supersecret").await;

    let (status, _, body) = server
        .post_with_cookie(&format!("/api/v1/alerts/{id}/acknowledge"), cookie.as_deref(), json!({}))
        .await;

    // Not 403. A manager probing ids must not be able to tell an employee in
    // another department from one who does not exist (ADR 0009) - the same
    // answer the drill-down gives.
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // And it really did not take effect.
    let state: String = sqlx::query_scalar("SELECT state::text FROM alerts WHERE id = $1")
        .bind(id)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(state, "open");

    server.close().await;
}

#[tokio::test]
async fn an_employee_cannot_read_the_feed() {
    let Some(server) = TestServer::start().await else { return };

    server.set_password("employee@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("employee@example.test", "supersecret").await;

    let (status, _) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "this is a manager's screen");

    let (status, _) = server.get_with_cookie("/api/v1/alerts", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn the_thresholds_are_the_administrators_and_they_move_the_line() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    server.add_admin("root@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;

    // Five hours of silence, under the twelve-hour default.
    last_seen(&server, "employee@example.test", now - Duration::hours(5)).await;
    assert_eq!(sweep(&server, now).await.raised, 0);

    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/alerts/thresholds",
            cookie.as_deref(),
            json!({ "alert_silence_hours": 4, "alert_overwork_factor": "1.5", "alert_open_day_hours": 16 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["alert_silence_hours"], 4);

    // The setting has to actually reach the sweep. The failure this guards is
    // a threshold read from `settings`, answered back on the form, and then
    // ignored in the rule in favour of a constant - which would look entirely
    // correct from the screen.
    let swept = sweep(&server, now).await;
    assert_eq!(swept.raised, 1, "five hours is an alert where the threshold is four");

    // And it is audited, like every other administrative change.
    let action: Option<String> = sqlx::query_scalar("SELECT action FROM audit_log WHERE action = 'alert.thresholds_changed'")
        .fetch_optional(&server.pool)
        .await
        .unwrap();
    assert_eq!(action.as_deref(), Some("alert.thresholds_changed"));

    server.close().await;
}

#[tokio::test]
async fn a_manager_cannot_change_the_thresholds() {
    let Some(server) = TestServer::start().await else { return };

    sqlx::query("UPDATE users SET role = 'manager' WHERE lower(email) = 'employee@example.test'")
        .execute(&server.pool)
        .await
        .unwrap();
    server.set_password("employee@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("employee@example.test", "supersecret").await;

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/alerts/thresholds",
            cookie.as_deref(),
            json!({ "alert_silence_hours": 1, "alert_overwork_factor": "1.1", "alert_open_day_hours": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "what the installation shouts about is the operator's decision");

    server.close().await;
}

#[tokio::test]
async fn a_threshold_outside_its_range_is_refused_with_a_sentence() {
    let Some(server) = TestServer::start().await else { return };

    server.add_admin("root@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;

    for (body, expected) in [
        (
            json!({ "alert_silence_hours": 0, "alert_overwork_factor": "1.5", "alert_open_day_hours": 16 }),
            "silence",
        ),
        (
            json!({ "alert_silence_hours": 12, "alert_overwork_factor": "1.0", "alert_open_day_hours": 16 }),
            "overwork",
        ),
        (
            json!({ "alert_silence_hours": 12, "alert_overwork_factor": "1.5", "alert_open_day_hours": 0 }),
            "open day",
        ),
    ] {
        let (status, _, answer) = server.put_with_cookie("/api/v1/alerts/thresholds", cookie.as_deref(), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // A sentence, not a Postgres constraint name reaching the screen.
        let message = answer["error"].as_str().unwrap_or_default();
        assert!(message.contains(expected), "expected `{expected}` in: {message}");
    }

    // A factor of exactly 1 is refused rather than accepted as "any day at
    // all is overwork": the check is `> 1`, and an off-by-one here would
    // alert on every ordinary eight-hour day in the installation.
    let factor: rust_decimal::Decimal = sqlx::query_scalar("SELECT alert_overwork_factor FROM settings WHERE singleton")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(factor.to_string(), "1.50", "nothing above was applied");

    server.close().await;
}

#[tokio::test]
async fn a_day_left_open_overnight_is_an_alert() {
    let Some(server) = TestServer::start().await else { return };

    let now = Utc::now();
    let date = a_recent_weekday();
    // An open day: kasl still running, no `ended_at`.
    let (status, _) = server
        .post_day(
            &server.token,
            json!({
                "date": date.to_string(),
                "started_at": (now - Duration::hours(20)).to_rfc3339(),
                "pauses": [],
                "tasks": []
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    last_seen(&server, "employee@example.test", now).await;

    let swept = sweep(&server, now).await;
    assert_eq!(swept.raised, 1);
    assert_eq!(open_rules(&server).await, vec!["day_not_closed"]);

    let observed: i64 = sqlx::query_scalar("SELECT observed_seconds FROM alerts WHERE rule = 'day_not_closed'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    // Within a second of twenty hours: the fixture's own clock moved between
    // building the timestamp and the sweep.
    assert!((observed - 20 * 3600).abs() <= 2, "expected about twenty hours, got {observed}");

    server.close().await;
}

#[tokio::test]
async fn the_feed_says_how_many_people_were_examined() {
    let Some(server) = TestServer::start().await else { return };

    server.add_admin("root@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;

    let (status, feed) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);

    // "Nothing found" alone cannot be told from "nobody was looked at", and
    // only one of those is good news - the same rule the signals band follows.
    assert_eq!(feed["open"], 0);
    assert_eq!(feed["people"], 2, "the employee and the administrator");
    assert_eq!(feed["thresholds"]["alert_silence_hours"], 12, "the screen can say what `too long` meant");

    server.close().await;
}

#[tokio::test]
async fn an_unknown_state_filter_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    server.add_admin("root@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;

    let (status, body) = server.get_with_cookie("/api/v1/alerts?state=everything", cookie.as_deref()).await;
    // Refused rather than quietly read as `open`: a client asking for a state
    // this server does not have is a client with a bug, and answering it with
    // a plausible list hides that.
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, _) = server.get_with_cookie("/api/v1/alerts?state=all", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);

    server.close().await;
}
