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
    let open = find("Sofia Reyes");
    assert_eq!(open["day_open"], true, "{open}");
    assert!(open["last_seen_at"].is_string(), "{open}");

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
    let steady = find("Tomas Verhoeven")["worked_seconds"].as_i64().unwrap();
    let fading = find("Lukas Brandt")["worked_seconds"].as_i64().unwrap();
    let long = find("Yusuf Demir")["worked_seconds"].as_i64().unwrap();
    assert!(fading < steady, "fading {fading} should be under steady {steady}");
    assert!(long > steady, "long {long} should be over steady {steady}");
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
