//! Backup and restore, against live databases.
//!
//! The claim under test is the only one that matters for a backup: **what
//! comes back is what went in**. So every test here fills one database through
//! the real API, dumps it, restores into a *second* empty database, and reads
//! the data back through the same endpoints - not by comparing files, which
//! would prove the dump is reproducible without proving it is usable.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::{TestDb, TestServer};

fn day(date: &str, agent_task_id: i64) -> Value {
    json!({
        "date": date,
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T18:00:00-03:00"),
        "pauses": [{
            "started_at": format!("{date}T12:00:00-03:00"),
            "ended_at": format!("{date}T12:30:00-03:00"),
            "duration_seconds": 1800,
            "manual": true,
            "reason": "lunch"
        }],
        "tasks": [{
            "agent_task_id": agent_task_id,
            "recorded_at": format!("{date}T17:00:00-03:00"),
            "name": "Ship the release",
            "comment": "went fine",
            "completeness": 100
        }]
    })
}

/// The schema version the server would migrate to - what a dump records.
fn schema_version() -> i64 {
    kasl_server::migrator().migrations.last().map(|m| m.version).unwrap_or_default()
}

/// Dumps `server` into memory.
async fn dump(server: &TestServer) -> Vec<u8> {
    let mut out = Vec::new();
    kasl_server::backup::dump(&server.pool, schema_version(), &mut out)
        .await
        .expect("the backup should be written");
    out
}

#[tokio::test]
async fn what_goes_in_comes_back_out() {
    let Some(server) = TestServer::start().await else { return };

    // A populated installation: two days through the real ingest path, an
    // administrator, and a department - so the restore has rows in most of the
    // tables and at least one foreign key between them.
    for (index, date) in ["2026-08-24", "2026-08-25"].iter().enumerate() {
        let (status, body) = server.post_day(&server.token, day(date, index as i64 + 1)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;
    let (status, _, body) = server
        .post_with_cookie("/api/v1/departments", admin.as_deref(), json!({ "name": "Engineering" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let file = dump(&server).await;
    assert!(!file.is_empty(), "the backup should not be empty");

    // A second, empty installation - the situation a real restore happens in.
    let Some(fresh) = TestDb::create().await else { return };
    let restored = kasl_server::backup::load(&fresh.pool, schema_version(), file.as_slice())
        .await
        .expect("the restore should succeed");
    assert!(restored.rows > 0, "the restore moved nothing: {restored:?}");

    // Read back through the API rather than by comparing rows: a backup is
    // only good if the server can serve what came out of it.
    let second = TestServer::wrap(fresh);
    let (_, cookie, _) = second.login("boss@example.test", "correct horse").await;
    let (status, body) = second
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-25", cookie.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let employee = body["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["email"] == "employee@example.test")
        .expect("the employee should have survived the restore");
    assert_eq!(employee["days_recorded"], 2, "{body}");
    assert_eq!(employee["worked_seconds"], 2 * (9 * 3600 - 1800), "{body}");

    // The details too, not just the totals: the pause and the task with the
    // words typed into them are the rows most easily lost in a restore.
    let ids: Vec<String> = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert!(!ids.is_empty());
    let (status, days) = second
        .get_with_cookie(
            &format!("/api/v1/users/{}/days?from=2026-08-24&to=2026-08-24", employee["id"].as_str().unwrap()),
            cookie.as_deref(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{days}");
    assert_eq!(days["days"][0]["pauses"][0]["reason"], "lunch", "{days}");
    assert_eq!(days["days"][0]["tasks"][0]["comment"], "went fine", "{days}");

    second.close().await;
}

#[tokio::test]
async fn a_department_and_its_manager_survive_each_other() {
    let Some(server) = TestServer::start().await else { return };

    // The cycle in this schema, and the defect that live use found while six
    // other tests passed: a person belongs to a department, and a department
    // names its manager. Neither can be inserted first with its column filled.
    // The earlier tests created a department nobody was in, so they never met
    // it.
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;

    let (status, body) = server.get_with_cookie("/api/v1/users", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let boss_id = body.as_array().unwrap().iter().find(|u| u["email"] == "boss@example.test").expect("the admin")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, _, body) = server
        .post_with_cookie("/api/v1/departments", admin.as_deref(), json!({ "name": "Engineering", "manager_id": boss_id }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let department_id = body["id"].as_str().unwrap().to_string();

    // Close the loop: the manager is also a member of the department they run.
    let (status, _, body) = server
        .put_with_cookie(
            &format!("/api/v1/users/{boss_id}/department"),
            admin.as_deref(),
            json!({ "department_id": department_id }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let file = dump(&server).await;

    let Some(fresh) = TestDb::create().await else { return };
    kasl_server::backup::load(&fresh.pool, schema_version(), file.as_slice())
        .await
        .expect("a department and its manager must restore together");

    // Both directions of the cycle, read back: the person is in the
    // department, and the department still names them.
    let restored: (Option<uuid::Uuid>, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT u.department_id, d.manager_id
         FROM users u JOIN departments d ON d.id = u.department_id
         WHERE u.email = 'boss@example.test'",
    )
    .fetch_one(&fresh.pool)
    .await
    .expect("the manager should be in their department");

    assert!(restored.0.is_some(), "the department membership was lost");
    assert_eq!(restored.1.map(|id| id.to_string()), Some(boss_id), "the department forgot who runs it");

    fresh.drop().await;
}

#[tokio::test]
async fn an_agent_can_still_deliver_after_a_restore() {
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.post_day(&server.token, day("2026-08-24", 1)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let file = dump(&server).await;

    let Some(fresh) = TestDb::create().await else { return };
    kasl_server::backup::load(&fresh.pool, schema_version(), file.as_slice())
        .await
        .expect("restore");

    // The token hash travelled with the agents table, so the machine in the
    // field keeps working without being re-enrolled. If this failed, every
    // employee would have to be issued a new token after a disaster - which is
    // the moment nobody has time for it.
    let second = TestServer::wrap(fresh);
    let (status, body) = second.post_day(&server.token, day("2026-08-26", 2)).await;
    assert_eq!(status, StatusCode::OK, "an agent's token must survive a restore: {body}");

    second.close().await;
}

#[tokio::test]
async fn a_restore_refuses_a_database_that_already_holds_people() {
    let Some(server) = TestServer::start().await else { return };
    let file = dump(&server).await;

    // The same database the dump came from: it still has its accounts.
    let error = kasl_server::backup::load(&server.pool, schema_version(), file.as_slice())
        .await
        .expect_err("restoring over data should be refused");

    // Refused rather than merged: every rule for what wins in a merge is wrong
    // for somebody, and picking one silently is the worst of them.
    assert!(error.to_string().contains("already holds"), "{error}");
}

#[tokio::test]
async fn a_backup_from_a_newer_server_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    // A dump claiming a schema this server has not reached yet.
    let mut file = Vec::new();
    kasl_server::backup::dump(&server.pool, schema_version() + 1, &mut file).await.expect("dump");

    let Some(fresh) = TestDb::create().await else { return };
    let error = kasl_server::backup::load(&fresh.pool, schema_version(), file.as_slice())
        .await
        .expect_err("a newer backup should be refused");

    // The failure this gate exists for is not an error - it is a restore that
    // appears to work while dropping columns the older schema has no place
    // for, which nobody notices until the data is needed.
    assert!(error.to_string().contains("newer server"), "{error}");
    fresh.drop().await;
}

#[tokio::test]
async fn a_file_that_is_not_a_backup_is_refused() {
    let Some(db) = TestDb::create().await else { return };

    for (bytes, expected) in [
        (&b""[..], "empty"),
        (&b"{}\n"[..], "not a kasl-server backup"),
        (&b"not json at all\n"[..], "not a kasl-server backup header"),
    ] {
        let error = kasl_server::backup::load(&db.pool, schema_version(), bytes)
            .await
            .expect_err("garbage should be refused");
        assert!(error.to_string().contains(expected), "expected `{expected}` in: {error}");
    }

    db.drop().await;
}

#[tokio::test]
async fn a_backup_names_a_table_this_server_does_not_have() {
    let Some(db) = TestDb::create().await else { return };

    // A hand-made file with a valid header and a hostile table name. The name
    // is the one string in the module that comes from outside it.
    let header = json!({
        "format": "kasl-server-backup",
        "schema_version": schema_version(),
        "server_version": "0.14.0",
        "taken_at": "2026-08-28T10:00:00Z"
    });
    let file = format!("{header}\n{}\n", json!({ "table": "users; DROP TABLE users", "rows": [{}] }));

    let error = kasl_server::backup::load(&db.pool, schema_version(), file.as_bytes())
        .await
        .expect_err("an unknown table should be refused");
    assert!(error.to_string().contains("does not have"), "{error}");

    // And the database it was aimed at is intact.
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&db.pool)
        .await
        .expect("users should still be there");
    assert_eq!(users, 0);

    db.drop().await;
}
