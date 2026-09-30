//! The demo: a fictional team on an empty database, and nothing anywhere else.
//!
//! What the demo shows is the dashboards, so these tests read them the way a
//! visitor would - signed in as the manager, the employee, the administrator -
//! and check that every state the dashboard can render is on screen.

mod support;

use axum::http::StatusCode;
use chrono::{Datelike, Duration, Utc};
use kasl_server::{
    backup,
    demo::{self, Status},
    model::UserRole,
};
use serde_json::json;
use support::{TestDb, TestServer};

/// Seeds the demo into a fresh database, or skips without one.
async fn demo_server() -> Option<(TestServer, demo::Seeded)> {
    let Some(db) = TestDb::create().await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return None;
    };
    let seeded = demo::seed(&db.pool, Utc::now()).await.expect("seeding an empty database should succeed");
    Some((TestServer::wrap(db), seeded))
}

/// The last seven days ending today: what the dashboard opens on.
fn this_week() -> String {
    let today = Utc::now().date_naive();
    format!("from={}&to={today}", today - Duration::days(6))
}

async fn signed_in(server: &TestServer, email: &str) -> String {
    let (status, cookie, body) = server.login(email, demo::PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{email} should sign in with the documented password: {body}");
    cookie.expect("a successful login sets the session cookie")
}

fn showcased(role: UserRole) -> String {
    demo::showcase()
        .into_iter()
        .find(|account| account.role == role)
        .map(|account| account.email)
        .expect("one account per role")
}

#[tokio::test]
async fn an_empty_database_becomes_the_demo_team() {
    let Some(db) = TestDb::create().await else { return };
    assert_eq!(demo::status(&db.pool).await.unwrap(), Status::Empty);

    let seeded = demo::seed(&db.pool, Utc::now()).await.unwrap();
    assert_eq!(seeded.departments, 3);
    assert_eq!(seeded.people, 12);
    // Eight weeks of weekdays for nine reporting people, minus the silent
    // one's last week and the odd sick day, is well over three hundred days.
    assert!(seeded.days > 300, "only {} days were written", seeded.days);

    assert_eq!(demo::status(&db.pool).await.unwrap(), Status::Demo);

    let server = TestServer::wrap(db);
    assert_eq!(server.count("departments").await, 3);
    assert_eq!(server.count("users").await, 12);
    assert_eq!(server.count("agents").await, 11, "everyone but the administrator has an agent");
    assert!(server.count("pauses").await > seeded.days as i64, "every day has at least a lunch break");
    assert!(server.count("tasks").await > seeded.days as i64, "every day logs more than one task");

    // The calendar, and the days people were away.
    //
    // Both are what the norm is for, and both were absent from the first
    // version of this seed without a single test noticing: `chance` takes a
    // percentage, and a nested `chance(4)`/`chance(2)` came to eight days in
    // ten thousand. The dashboard drew twelve rows at a full norm and looked
    // entirely plausible. A count is the only thing that says otherwise.
    assert_eq!(seeded.calendar_days, 3, "a holiday, a short day and a working weekend");
    assert_eq!(server.count("calendar_days").await, 3);

    let away: i64 = server.scalar("SELECT count(*) FROM workdays WHERE kind <> 'work'").await;
    assert!(away > 0, "the demo has to show what a day off looks like, and seeded none");

    let part_time: i64 = server.scalar("SELECT count(*) FROM users WHERE work_rate <> 1").await;
    assert_eq!(part_time, 1, "exactly one person is on part time, so the norm has something to divide");

    // The seeding is a thing the server did, and the audit log says so.
    let recorded: i64 = server
        .scalar("SELECT count(*) FROM audit_log WHERE action = 'demo.seeded' AND actor_id IS NULL")
        .await;
    assert_eq!(recorded, 1);

    // What the web UI reads before anyone signs in.
    let (status, body) = server.get_with_cookie("/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["demo"], true, "{body}");
}

#[tokio::test]
async fn a_real_installation_is_not_a_demo() {
    // One provisioned employee: somebody's actual server.
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.get_with_cookie("/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["demo"], false, "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/demo/accounts", None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a real installation must not list its people to a stranger: {body}"
    );
}

#[tokio::test]
async fn a_populated_database_is_refused_and_left_alone() {
    let Some(server) = TestServer::start().await else { return };
    assert_eq!(demo::status(&server.pool).await.unwrap(), Status::Populated { accounts: 1 });

    let error = demo::seed(&server.pool, Utc::now()).await.unwrap_err().to_string();
    assert!(error.contains("already holds 1 accounts"), "the refusal should say what is in the way: {error}");

    // Nothing was written - not a department, not the demo mark.
    assert_eq!(server.count("users").await, 1);
    assert_eq!(server.count("departments").await, 0);
    assert_eq!(server.count("workdays").await, 0);
    let demo: bool = server.scalar("SELECT demo FROM settings WHERE singleton").await;
    assert!(!demo, "a refused seed must not mark the installation as a demo");
}

#[tokio::test]
async fn a_demo_is_not_seeded_twice() {
    let Some((server, _)) = demo_server().await else { return };

    let error = demo::seed(&server.pool, Utc::now()).await.unwrap_err().to_string();
    assert!(error.contains("already holds the demo team"), "{error}");
    assert_eq!(server.count("users").await, 12, "the second attempt must not add anybody");
}

#[tokio::test]
async fn every_showcased_account_signs_in_and_is_who_it_says() {
    let Some((server, _)) = demo_server().await else { return };

    let accounts = demo::showcase();
    assert_eq!(accounts.len(), 3);
    for account in accounts {
        let cookie = signed_in(&server, &account.email).await;
        let (status, me) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["email"], account.email);
        assert_eq!(me["display_name"], account.display_name);
        assert_eq!(me["role"], serde_json::to_value(account.role).unwrap());
    }
}

#[tokio::test]
async fn the_login_screen_can_list_the_accounts() {
    let Some((server, _)) = demo_server().await else { return };

    let (status, body) = server.get_with_cookie("/api/v1/demo/accounts", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["password"], demo::PASSWORD);
    let accounts = body["accounts"].as_array().expect("a list of accounts");
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[0]["role"], "manager", "the manager's dashboard is what the demo is for: {body}");
    for account in accounts {
        assert!(account["email"].as_str().unwrap().ends_with("@example.com"), "{account}");
    }
}

#[tokio::test]
async fn the_manager_sees_their_department_with_hours_in_it() {
    let Some((server, _)) = demo_server().await else { return };

    let cookie = signed_in(&server, &showcased(UserRole::Manager)).await;
    let (status, team) = server.get_with_cookie(&format!("/api/v1/team/days?{}", this_week()), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{team}");

    let members = team["members"].as_array().unwrap();
    // Engineering: the manager and four engineers, nobody from elsewhere.
    assert_eq!(members.len(), 5, "{team}");
    assert!(members.iter().all(|m| m["department"] == "Engineering"), "{team}");

    // A week of history for everyone in it: a dashboard of zeros sells nothing.
    for member in members {
        assert!(
            member["worked_seconds"].as_i64().unwrap() > 0,
            "{} worked nothing this week: {member}",
            member["display_name"]
        );
        assert!(member["days_recorded"].as_i64().unwrap() >= 3, "{member}");
        assert_eq!(member["agents"], 1, "{member}");
    }
}

#[tokio::test]
async fn every_state_the_dashboard_can_show_is_on_screen() {
    let Some((server, _)) = demo_server().await else { return };

    let cookie = signed_in(&server, &showcased(UserRole::Admin)).await;
    let (status, team) = server.get_with_cookie(&format!("/api/v1/team/days?{}", this_week()), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{team}");
    let members = team["members"].as_array().unwrap();
    assert_eq!(members.len(), 12, "the administrator sees everyone: {team}");

    let find = |name: &str| {
        members
            .iter()
            .find(|m| m["display_name"] == name)
            .unwrap_or_else(|| panic!("{name} is not on the dashboard"))
    };

    // Working right now.
    //
    // `day_open` is deliberately not asserted here. The flag compares the
    // employee's own local date against the server's `current_date`, and this
    // person is three hours behind UTC: for three hours of every day her open
    // day sits on yesterday's date as far as the server is concerned, and the
    // flag is false while she is very much at work (ADR 0003 accepts this -
    // the alternative needs a per-person time zone the server does not store).
    // Asserting it made this test pass or fail by the hour of the run.
    //
    // What is asserted instead is what the demo actually has to produce: a day
    // with no end on it, and a machine that was heard from.
    let open = find("Sofia Reyes");
    assert!(open["last_seen_at"].is_string(), "{open}");
    let running: i64 = sqlx::query_scalar("SELECT count(*) FROM workdays w JOIN users u ON u.id = w.user_id WHERE u.email = $1 AND w.ended_at IS NULL")
        .bind("sofia.reyes@example.com")
        .fetch_one(&server.pool)
        .await
        .expect("the demo's open day is in the database");
    assert_eq!(running, 1, "the demo seeds exactly one day still running");

    // Went quiet a week ago: nothing this week, and the server has not heard
    // from the machine in days.
    let silent = find("Jonas Petit");
    assert_eq!(silent["days_recorded"], 0, "{silent}");
    assert_eq!(silent["agents"], 1, "{silent}");
    let last_seen: chrono::DateTime<Utc> = silent["last_seen_at"].as_str().expect("was seen once").parse().unwrap();
    assert!(Utc::now() - last_seen > Duration::days(6), "{silent}");

    // Has an agent, never sent anything.
    let never = find("Hana Kowalski");
    assert_eq!(never["days_recorded"], 0, "{never}");
    assert_eq!(never["agents"], 1, "{never}");
    assert!(never["last_seen_at"].is_null(), "{never}");

    // Runs the installation, has no agent at all.
    let admin = find("Sam Whitfield");
    assert_eq!(admin["agents"], 0, "{admin}");
    assert!(admin["department"].is_null(), "{admin}");

    // The one whose hours are shrinking, and the one who works too long, are
    // both visibly different from the steady ones this week.
    //
    // Compared per day, not per week. A week's total is the length of a day
    // times the days that were worked, and since the demo seeds days off the
    // two factors move independently: the long-hours person had two days sick
    // this week and his weekly total came out under the steady person's, which
    // failed an assertion about how long his days are. Days recorded is the
    // divisor the claim actually needs.
    let per_day = |name: &str| {
        let row = find(name);
        let days = row["days_recorded"].as_i64().unwrap();
        assert!(days > 0, "{name} recorded nothing this week: {row}");
        row["worked_seconds"].as_i64().unwrap() / days
    };

    let steady = per_day("Tomas Verhoeven");
    let fading = per_day("Lukas Brandt");
    let long = per_day("Yusuf Demir");
    assert!(fading < steady, "a fading day {fading} should be under a steady one {steady}");
    assert!(long > steady, "a long day {long} should be over a steady one {steady}");
}

#[tokio::test]
async fn every_live_status_the_dashboard_can_show_is_on_screen() {
    let Some((server, _)) = demo_server().await else { return };

    let cookie = signed_in(&server, &showcased(UserRole::Admin)).await;
    let (status, live) = server.get_with_cookie("/api/v1/team/live", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{live}");

    let (_, team) = server.get_with_cookie(&format!("/api/v1/team/days?{}", this_week()), Some(&cookie)).await;
    let id_of = |name: &str| {
        team["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["display_name"] == name)
            .unwrap_or_else(|| panic!("{name} is not on the dashboard"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let status_of = |name: &str| {
        let id = id_of(name);
        live["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["user_id"] == id.as_str())
            .unwrap_or_else(|| panic!("{name} has no live row"))["status"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // The point of seeding pulses at all: a visitor opening the demo should
    // see the status column doing its job, not a page of "unknown" that makes
    // the feature look broken.
    assert_eq!(status_of("Sofia Reyes"), "working", "the open day is at the keyboard: {live}");
    assert_eq!(status_of("Aiko Tanaka"), "paused", "someone has to be on a break: {live}");
    // Their agent is up and reporting, they are simply done for the day - the
    // distinction between `idle` and `offline` only shows if both are present.
    assert_eq!(status_of("Tomas Verhoeven"), "idle", "{live}");
    // Seeded with a deliberately old pulse: the machine was answering this
    // morning and has stopped. That is the row a manager should look at first,
    // and it only exists if the demo carries one - "no pulse at all" reads as
    // `unknown`, which says nothing about the person.
    assert_eq!(status_of("Lukas Brandt"), "offline", "{live}");
    // Never sent one. Distinct from offline on purpose: nothing here is
    // evidence about the person, only about the installation.
    assert_eq!(status_of("Hana Kowalski"), "unknown", "{live}");
}

#[tokio::test]
async fn the_demo_pulses_stay_fresh() {
    let Some((server, _)) = demo_server().await else { return };

    let cookie = signed_in(&server, &showcased(UserRole::Admin)).await;
    let statuses = |live: &serde_json::Value| -> Vec<String> {
        live["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["status"].as_str().unwrap().to_string())
            .collect()
    };

    // A pulse is believed for three minutes and the demo is seeded once, so
    // without a refresh the whole live column would turn "offline" while the
    // first visitor was still reading the page - and the milestone would be
    // invisible on the one installation built to show it off.
    server
        .execute("UPDATE agents SET heartbeat_received_at = now() - interval '1 day' WHERE heartbeat_state IS NOT NULL")
        .await;
    let (_, gone) = server.get_with_cookie("/api/v1/team/live", Some(&cookie)).await;
    assert!(!statuses(&gone).contains(&"working".to_string()), "aged out, as the fixture intends: {gone}");

    let refreshed = demo::refresh_pulses(&server.pool).await.expect("the refresh should succeed");
    assert!(refreshed > 0, "the demo seeds pulses, so some must have been re-stamped");

    let (_, live) = server.get_with_cookie("/api/v1/team/live", Some(&cookie)).await;
    let after = statuses(&live);
    assert!(after.contains(&"working".to_string()), "a refreshed pulse is believed again: {live}");

    // What the refresh must not do: invent a pulse for an agent that never
    // sent one. Which people are live was decided at seed time, and a refresh
    // that widened it would put someone at a keyboard who is not there.
    assert!(after.contains(&"unknown".to_string()), "silence must stay silent: {live}");
}

#[tokio::test]
async fn a_demo_made_by_another_version_is_generated_again() {
    // The class the pulse (v0.17.1) and the calendar (v0.21) were each patched
    // for, one field at a time: a stand upgraded to a new image keeps the old
    // generator's team, without whatever the new version was released to
    // show. The remedy is the stand being born again, not a third patch.
    let Some((server, _)) = demo_server().await else { return };
    let now = Utc::now();
    assert_eq!(
        demo::regeneration_due(&server.pool, now).await.unwrap(),
        None,
        "a demo this version just seeded is current"
    );

    // What an upgraded stand looks like: made by an earlier version, and
    // missing what that version did not generate.
    // Not the running version, whatever that is when this runs: the version
    // this test was written in is the one it must not pick.
    server.execute("UPDATE settings SET demo_seeded_by = '0.1.0'").await;
    server.execute("DELETE FROM day_notes").await;
    server.execute("DELETE FROM calendar_days").await;
    server
        .execute("UPDATE agents SET heartbeat_state = NULL, heartbeat_at = NULL, heartbeat_received_at = NULL, demo_pulse_age_seconds = NULL")
        .await;
    assert_eq!(
        demo::regeneration_due(&server.pool, now).await.unwrap(),
        Some(demo::Regenerate::OtherVersion {
            seeded_by: Some("0.1.0".to_string())
        })
    );

    demo::reseed(&server.pool, now).await.expect("a demo should regenerate");
    assert_eq!(demo::regeneration_due(&server.pool, now).await.unwrap(), None, "and is then current");
    let notes: i64 = server.scalar("SELECT count(*) FROM day_notes").await;
    let calendar: i64 = server.scalar("SELECT count(*) FROM calendar_days").await;
    let pulses: i64 = server.scalar("SELECT count(*) FROM agents WHERE heartbeat_state IS NOT NULL").await;
    assert!(
        notes > 0 && calendar > 0 && pulses > 0,
        "{notes} notes, {calendar} calendar days, {pulses} pulses"
    );
}

#[tokio::test]
async fn a_demo_from_before_the_version_was_recorded_is_generated_again() {
    // Every demo stand in existence the day this shipped: `demo_seeded_by`
    // arrived empty. It has to count as another version, not as this one.
    let Some((server, _)) = demo_server().await else { return };
    server.execute("UPDATE settings SET demo_seeded_by = NULL").await;
    assert_eq!(
        demo::regeneration_due(&server.pool, Utc::now()).await.unwrap(),
        Some(demo::Regenerate::OtherVersion { seeded_by: None })
    );
}

#[tokio::test]
async fn the_showcased_employee_finds_a_note_on_a_day_to_come() {
    // The note the feature is keyed by date for: leave approved on a day that
    // has not happened and has no workday row. On the account a visitor is
    // offered, so it is the first note anybody sees.
    let Some((server, seeded)) = demo_server().await else { return };
    assert_eq!(seeded.notes, 3);
    let cookie = signed_in(&server, &showcased(UserRole::Employee)).await;

    let today = Utc::now().date_naive();
    let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let (from, to) = (monday + Duration::days(7), monday + Duration::days(13));
    let (status, week) = server.get_with_cookie(&format!("/api/v1/me/days?from={from}&to={to}"), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{week}");
    let notes = week["notes"].as_array().expect("the week answers its notes");
    assert_eq!(notes.len(), 1, "{week}");
    assert_eq!(notes[0]["date"], (monday + Duration::days(11)).to_string(), "on next week's Friday");
    assert_eq!(notes[0]["author"], "Priya Raman");
    assert!(week["days"].as_array().unwrap().is_empty(), "a day to come has no workday: {week}");

    // And the bell says so, with the words and a link to that week.
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", Some(&cookie)).await;
    let told = inbox["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|notice| notice["kind"] == "note.added")
        .count();
    assert_eq!(told, 2, "both of the employee's notes are behind the bell: {inbox}");

    // A real kasl pointed at the demo is told them too.
    let (status, queue) = server.get_with_header("/api/v1/agent/notifications", Some("Bearer demo-tomas")).await;
    assert_eq!(status, StatusCode::OK, "{queue}");
    assert!(
        queue["notifications"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice["kind"] == "note.added" && notice["body"].as_str().unwrap().contains("day off")),
        "{queue}"
    );
}

#[tokio::test]
async fn refreshing_the_demo_does_not_heal_the_agent_that_stopped() {
    let Some((server, _)) = demo_server().await else { return };
    let cookie = signed_in(&server, &showcased(UserRole::Admin)).await;

    // The defect this guards, and the reason the refresh is not a plain
    // "set everything to now()": the row a manager is meant to notice - a
    // machine that has stopped answering - would be quietly healed on the
    // first tick, and the demo would show a team where nothing is ever wrong.
    for _ in 0..3 {
        demo::refresh_pulses(&server.pool).await.expect("the refresh should succeed");
    }

    let (_, live) = server.get_with_cookie("/api/v1/team/live", Some(&cookie)).await;
    let statuses: Vec<&str> = live["members"].as_array().unwrap().iter().map(|m| m["status"].as_str().unwrap()).collect();
    assert!(statuses.contains(&"offline"), "the stopped agent must stay stopped: {live}");
    assert!(statuses.contains(&"working"), "and the live ones must stay live: {live}");
}

#[tokio::test]
async fn the_employee_sees_their_own_week_in_detail() {
    let Some((server, _)) = demo_server().await else { return };

    let cookie = signed_in(&server, &showcased(UserRole::Employee)).await;
    let (status, week) = server.get_with_cookie(&format!("/api/v1/me/days?{}", this_week()), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{week}");

    let days = week["days"].as_array().unwrap();
    assert!(days.len() >= 3, "{week}");
    for day in days {
        assert!(!day["pauses"].as_array().unwrap().is_empty(), "every day has its lunch: {day}");
        assert!(!day["tasks"].as_array().unwrap().is_empty(), "every day logs a task: {day}");
        // Weekdays only: a weekend row would say the team works seven days.
        let date: chrono::NaiveDate = day["date"].as_str().unwrap().parse().unwrap();
        assert!(!matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun), "{day}");
    }

    // The manager's screen answers 403, as it does for any employee.
    let (status, _) = server.get_with_cookie(&format!("/api/v1/team/days?{}", this_week()), Some(&cookie)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_real_agent_can_report_into_the_demo() {
    // The tokens are documented so a kasl can be pointed at the demo and its
    // days show up next to the invented ones.
    let Some((server, _)) = demo_server().await else { return };

    let today = Utc::now().date_naive();
    let (status, body) = server
        .post_day(
            "demo-tomas",
            json!({
                "date": today,
                "started_at": format!("{today}T09:00:00+02:00"),
                "ended_at": format!("{today}T12:30:00+02:00"),
                "tasks": [{ "agent_task_id": 9001, "recorded_at": format!("{today}T12:29:00+02:00"), "name": "From a real agent", "completeness": 100 }]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tasks"], 1, "{body}");
}

#[tokio::test]
async fn two_demos_started_on_the_same_day_show_the_same_numbers() {
    // A screenshot from one demo must be reproducible on another.
    let Some(one) = TestDb::create().await else { return };
    let Some(two) = TestDb::create().await else { return };
    let now = Utc::now();
    demo::seed(&one.pool, now).await.unwrap();
    demo::seed(&two.pool, now).await.unwrap();

    async fn totals(server: &TestServer, cookie: String) -> Vec<(String, i64, i64)> {
        // The whole eight weeks, inside the range a request may ask for.
        let today = Utc::now().date_naive();
        let range = format!("from={}&to={today}", today - Duration::days(70));
        let (status, team) = server.get_with_cookie(&format!("/api/v1/team/days?{range}"), Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{team}");
        team["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["email"].to_string(),
                    m["worked_seconds"].as_i64().unwrap(),
                    m["days_recorded"].as_i64().unwrap(),
                )
            })
            .collect()
    }

    let one = TestServer::wrap(one);
    let two = TestServer::wrap(two);
    let admin = showcased(UserRole::Admin);
    let first = totals(&one, signed_in(&one, &admin).await).await;
    let second = totals(&two, signed_in(&two, &admin).await).await;
    assert_eq!(first, second);
    assert!(first.iter().any(|(_, worked, _)| *worked > 0));

    one.close().await;
    two.close().await;
}

#[tokio::test]
async fn a_restored_demo_is_still_a_demo() {
    let Some((source, _)) = demo_server().await else { return };
    let schema = kasl_server::migrator().migrations.last().map(|m| m.version).unwrap_or_default();

    let mut file = Vec::new();
    backup::dump(&source.pool, schema, &mut file).await.unwrap();
    source.close().await;

    let Some(target) = TestDb::create().await else { return };
    backup::load(&target.pool, schema, std::io::Cursor::new(file)).await.unwrap();
    // The label travels with the data: a restored demo must not present its
    // invented people as a real team.
    assert_eq!(demo::status(&target.pool).await.unwrap(), Status::Demo);
    target.drop().await;
}

#[tokio::test]
async fn a_backup_from_before_the_demo_existed_still_restores() {
    // A settings row written by an older server has no `demo` key. The
    // column is NOT NULL, and a restore that failed on a perfectly good older
    // file would be the worst possible moment to find that out.
    let Some(source) = TestServer::start().await else { return };
    let schema = kasl_server::migrator().migrations.last().map(|m| m.version).unwrap_or_default();

    let mut file = Vec::new();
    backup::dump(&source.pool, schema, &mut file).await.unwrap();
    source.close().await;

    let text = String::from_utf8(file).unwrap();
    let older: String = text
        .lines()
        .map(|line| {
            if !line.contains(r#""table":"settings""#) {
                return line.to_string();
            }
            let mut chunk: serde_json::Value = serde_json::from_str(line).unwrap();
            for row in chunk["rows"].as_array_mut().unwrap() {
                row.as_object_mut().unwrap().remove("demo").expect("the dump carries the column");
            }
            chunk.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    let Some(target) = TestDb::create().await else { return };
    backup::load(&target.pool, schema, std::io::Cursor::new(older.into_bytes()))
        .await
        .expect("an older file must restore");
    assert_eq!(demo::status(&target.pool).await.unwrap(), Status::Populated { accounts: 1 });
    target.drop().await;
}

#[tokio::test]
async fn every_alert_rule_fires_on_the_demo_and_nothing_else_does() {
    let Some((server, _)) = demo_server().await else { return };

    let swept = kasl_server::alerts::sweep(&server.pool, Utc::now(), &kasl_server::webhooks::Webhooks::default())
        .await
        .expect("the sweep should run");
    assert!(swept.raised > 0, "the demo exists to show these; a demo with no alerts shows none of them");

    let rules: Vec<String> = sqlx::query_scalar("SELECT DISTINCT rule::text FROM alerts WHERE state = 'open' ORDER BY rule::text")
        .fetch_all(&server.pool)
        .await
        .expect("the alerts should be readable");

    // All three, because a demo that shows one of them is a demo of one third
    // of the milestone - and which ones fire is a property of the fictional
    // days, not of the rules. The first live run of this version raised only
    // `no_agent_data`: the longest day in the demo was 10.7 h against a 12 h
    // bar, and the only open day was three hours old.
    for rule in ["no_agent_data", "overwork", "day_not_closed"] {
        assert!(rules.iter().any(|r| r == rule), "no `{rule}` on the demo: got {rules:?}");
    }

    // And the other half of the claim, which is the one a live run actually
    // broke: the feed has to be readable. Nine of twelve people were flagged
    // silent before the silence rule learnt to read the pulse as well as the
    // token - true of nobody, and a wall of noise is how a manager learns to
    // ignore the column.
    let people: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE active")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM alerts WHERE state = 'open'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert!(open * 2 < people, "{open} alerts across {people} people is a wall, not a signal",);
}

#[tokio::test]
async fn a_demo_whose_history_stopped_is_generated_again() {
    let Some((server, _)) = demo_server().await else { return };

    let now = Utc::now();
    assert!(
        !demo::history_is_stale(&server.pool, now).await.unwrap(),
        "a demo seeded a moment ago is current",
    );
    // Nor is it stale over a weekend: the fictional team works weekdays, so on
    // a Sunday its newest day is rightly Friday's. A threshold of one day
    // would regenerate the whole stand every Sunday morning.
    assert!(!demo::history_is_stale(&server.pool, now + Duration::days(2)).await.unwrap());

    // A fortnight on, every row of the dashboard reads "no days recorded for
    // 16 days". That is a truthful description of the data and a false one of
    // the product, which is what this exists to prevent.
    assert!(demo::history_is_stale(&server.pool, now + Duration::days(16)).await.unwrap());

    let before: Vec<String> = sqlx::query_scalar("SELECT email FROM users ORDER BY email")
        .fetch_all(&server.pool)
        .await
        .unwrap();
    let newest_before: chrono::NaiveDate = sqlx::query_scalar("SELECT max(date) FROM workdays").fetch_one(&server.pool).await.unwrap();

    // Regenerated for a "today" a fortnight from now.
    let later = now + Duration::days(16);
    let seeded = demo::reseed(&server.pool, later).await.expect("a demo should regenerate");
    assert_eq!(seeded.people, 12);

    let newest_after: chrono::NaiveDate = sqlx::query_scalar("SELECT max(date) FROM workdays").fetch_one(&server.pool).await.unwrap();
    assert!(newest_after > newest_before, "the history has to reach the new today");
    assert!(
        !demo::history_is_stale(&server.pool, later).await.unwrap(),
        "and it is not stale against the day it was generated for",
    );

    // The same fictional people, not a second team beside the first: the
    // failure worth guarding is a regeneration that appends rather than
    // replaces, which would leave a dashboard of twenty-four.
    let after: Vec<String> = sqlx::query_scalar("SELECT email FROM users ORDER BY email")
        .fetch_all(&server.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(server.count("users").await, 12);
    assert_eq!(server.count("departments").await, 3);

    // And it is still a demo, so the banner keeps saying nothing here is real.
    let demo_flag: bool = sqlx::query_scalar("SELECT demo FROM settings WHERE singleton")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert!(demo_flag);
}

#[tokio::test]
async fn a_real_installation_is_never_regenerated() {
    let Some(server) = support::TestServer::start().await else { return };

    // The guard that matters: `KASL_DEMO` left in a file after a trial must
    // not be able to delete a real team. `seed` already refuses a populated
    // database; this is the same refusal on the path that deletes first.
    let error = demo::reseed(&server.pool, Utc::now()).await.unwrap_err().to_string();
    assert!(error.contains("not one"), "{error}");

    // And the account it holds is still there.
    assert_eq!(server.count("users").await, 1);

    server.close().await;
}
