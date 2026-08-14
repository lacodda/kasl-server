//! Exercises the migrations against a real PostgreSQL.
//!
//! The schema is a contract with two parties that cannot both be checked by
//! reading the SQL: kasl agents upload into it, and every dashboard reads out
//! of it. These tests assert the guarantees that upload depends on - one day
//! per person per date, a re-upload landing on the same row, a departed
//! employee taking their rows with them - by running them.
//!
//! Skipped unless `DATABASE_URL` is set, so `cargo test` stays green on a
//! machine without a database; CI runs them with a Postgres service.

mod support;

use sqlx::PgPool;
use support::with_db;
use uuid::Uuid;

/// Inserts a user and returns its id.
async fn insert_user(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO users (email, display_name) VALUES ($1, $2) RETURNING id")
        .bind(email)
        .bind("Test Person")
        .fetch_one(pool)
        .await
        .expect("failed to insert a user")
}

async fn insert_workday(pool: &PgPool, user: Uuid, date: &str) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar("INSERT INTO workdays (user_id, date, started_at) VALUES ($1, $2::date, $2::date + time '09:00' AT TIME ZONE 'UTC') RETURNING id")
        .bind(user)
        .bind(date)
        .fetch_one(pool)
        .await
}

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    with_db(|pool| async move {
        // A second run must be a no-op: the server migrates on every startup.
        kasl_server::migrator().run(&pool).await.expect("re-running migrations must be a no-op");

        let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name")
            .fetch_all(&pool)
            .await
            .expect("failed to list tables");

        for expected in ["agents", "pauses", "reports", "tags", "task_tags", "tasks", "users", "workdays"] {
            assert!(tables.iter().any(|t| t == expected), "table `{expected}` is missing; got {tables:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_person_has_one_workday_per_date() {
    with_db(|pool| async move {
        let user = insert_user(&pool, "one-day@example.test").await;
        insert_workday(&pool, user, "2026-08-14").await.expect("the first workday should insert");

        let duplicate = insert_workday(&pool, user, "2026-08-14").await;
        assert!(duplicate.is_err(), "a second workday on the same date must be rejected");

        // The same date for a different person is a different day.
        let colleague = insert_user(&pool, "colleague@example.test").await;
        insert_workday(&pool, colleague, "2026-08-14")
            .await
            .expect("another person's day on the same date must be allowed");
    })
    .await;
}

#[tokio::test]
async fn emails_are_unique_regardless_of_case() {
    with_db(|pool| async move {
        insert_user(&pool, "Person@Example.test").await;

        let duplicate = sqlx::query("INSERT INTO users (email, display_name) VALUES ($1, $2)")
            .bind("person@example.test")
            .bind("Impostor")
            .execute(&pool)
            .await;
        assert!(duplicate.is_err(), "logins must not differ only by letter case");
    })
    .await;
}

#[tokio::test]
async fn an_upload_of_the_same_task_updates_one_row() {
    with_db(|pool| async move {
        let user = insert_user(&pool, "tasks@example.test").await;

        // The agent identifies a task by its own row id; a re-upload of a day
        // must correct the row it already sent, not add a second one.
        for completeness in [40_i16, 100] {
            sqlx::query(
                "INSERT INTO tasks (user_id, agent_task_id, agent_group_id, date, recorded_at, name, completeness)
                 VALUES ($1, 7, 7, date '2026-08-14', now(), 'Ship the schema', $2)
                 ON CONFLICT (user_id, agent_task_id) DO UPDATE SET completeness = EXCLUDED.completeness",
            )
            .bind(user)
            .bind(completeness)
            .execute(&pool)
            .await
            .expect("upsert should succeed");
        }

        let rows: Vec<(i32,)> = sqlx::query_as("SELECT completeness::int FROM tasks WHERE user_id = $1")
            .bind(user)
            .fetch_all(&pool)
            .await
            .expect("failed to read tasks");
        assert_eq!(rows, [(100,)], "the re-upload should have updated the single row");
    })
    .await;
}

#[tokio::test]
async fn a_workday_owns_its_pauses() {
    with_db(|pool| async move {
        let user = insert_user(&pool, "pauses@example.test").await;
        let workday = insert_workday(&pool, user, "2026-08-14").await.expect("workday");

        sqlx::query("INSERT INTO pauses (workday_id, started_at, ended_at, duration_seconds) VALUES ($1, now(), now() + interval '20 minutes', 1200)")
            .bind(workday)
            .execute(&pool)
            .await
            .expect("failed to insert a pause");

        // Deleting the day takes its pauses; a pause without a day is not a
        // state any reader can make sense of.
        sqlx::query("DELETE FROM workdays WHERE id = $1")
            .bind(workday)
            .execute(&pool)
            .await
            .expect("failed to delete the workday");

        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM pauses")
            .fetch_one(&pool)
            .await
            .expect("failed to count pauses");
        assert_eq!(left, 0, "pauses must not outlive their workday");
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_takes_their_data() {
    with_db(|pool| async move {
        let user = insert_user(&pool, "departed@example.test").await;
        let workday = insert_workday(&pool, user, "2026-08-14").await.expect("workday");
        sqlx::query("INSERT INTO pauses (workday_id, started_at) VALUES ($1, now())")
            .bind(workday)
            .execute(&pool)
            .await
            .expect("pause");
        sqlx::query("INSERT INTO agents (user_id, name, token_hash) VALUES ($1, 'laptop', 'hash')")
            .bind(user)
            .execute(&pool)
            .await
            .expect("agent");
        sqlx::query("INSERT INTO reports (user_id, kind, period_start, submitted_at) VALUES ($1, 'daily', date '2026-08-14', now())")
            .bind(user)
            .execute(&pool)
            .await
            .expect("report");

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(&pool)
            .await
            .expect("failed to delete the user");

        for (table, sql) in [
            ("workdays", "SELECT count(*) FROM workdays"),
            ("pauses", "SELECT count(*) FROM pauses"),
            ("agents", "SELECT count(*) FROM agents"),
            ("reports", "SELECT count(*) FROM reports"),
        ] {
            let left: i64 = sqlx::query_scalar(sql).fetch_one(&pool).await.expect("failed to count rows");
            assert_eq!(left, 0, "`{table}` still holds rows of a deleted user");
        }
    })
    .await;
}

#[tokio::test]
async fn impossible_intervals_are_rejected() {
    with_db(|pool| async move {
        let user = insert_user(&pool, "checks@example.test").await;
        let workday = insert_workday(&pool, user, "2026-08-14").await.expect("workday");

        let backwards = sqlx::query("UPDATE workdays SET ended_at = started_at - interval '1 hour' WHERE id = $1")
            .bind(workday)
            .execute(&pool)
            .await;
        assert!(backwards.is_err(), "a day cannot end before it starts");

        let negative = sqlx::query("INSERT INTO pauses (workday_id, started_at, duration_seconds) VALUES ($1, now(), -1)")
            .bind(workday)
            .execute(&pool)
            .await;
        assert!(negative.is_err(), "a pause cannot last a negative time");

        let over_complete = sqlx::query(
            "INSERT INTO tasks (user_id, agent_task_id, agent_group_id, date, recorded_at, name, completeness)
             VALUES ($1, 1, 1, date '2026-08-14', now(), 'Overachieve', 101)",
        )
        .bind(user)
        .execute(&pool)
        .await;
        assert!(over_complete.is_err(), "completeness above 100 must be rejected");
    })
    .await;
}

#[tokio::test]
async fn updated_at_follows_the_row() {
    with_db(|pool| async move {
        let user = insert_user(&pool, "touched@example.test").await;

        let before: (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) = sqlx::query_as("SELECT created_at, updated_at FROM users WHERE id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .expect("failed to read timestamps");
        assert_eq!(before.0, before.1, "a fresh row should carry equal timestamps");

        sqlx::query("UPDATE users SET display_name = 'Renamed' WHERE id = $1")
            .bind(user)
            .execute(&pool)
            .await
            .expect("failed to update");

        let after: (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) = sqlx::query_as("SELECT created_at, updated_at FROM users WHERE id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .expect("failed to read timestamps");
        assert_eq!(after.0, before.0, "created_at must not move");
        assert!(after.1 > before.1, "updated_at must advance on write");
    })
    .await;
}
