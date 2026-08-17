//! Exercises the history import against a real agent database and a real
//! server database.
//!
//! The fixture is built with the agent's own schema - the table and column
//! names from kasl's migrations, wall-clock text with no offset - rather than a
//! shape convenient to read. An import that only works on a tidy file is not an
//! import: what arrives is whatever a year of someone's work left behind.
//!
//! Skipped unless `DATABASE_URL` is set; CI runs them with a Postgres service.

mod support;

use chrono::FixedOffset;
use kasl_server::import;
use support::TestServer;

/// Builds a SQLite database shaped like an agent's, at the given path.
///
/// Written as raw SQL matching kasl's migrations, not through any shared code:
/// if the agent's schema drifts, this fixture should stop matching it and the
/// tests should say so.
fn agent_db(path: &std::path::Path) {
    let connection = rusqlite::Connection::open(path).expect("the fixture database should open");
    connection
        .execute_batch(
            "CREATE TABLE workdays (id INTEGER PRIMARY KEY, date DATE NOT NULL UNIQUE, start TIMESTAMP NOT NULL, end TIMESTAMP, notes TEXT);
             CREATE TABLE pauses (id INTEGER PRIMARY KEY, start TIMESTAMP NOT NULL, end TIMESTAMP, duration INTEGER);
             CREATE TABLE tasks (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL DEFAULT 0, timestamp TIMESTAMP,
                                 name TEXT NOT NULL, comment TEXT, completeness INTEGER NOT NULL DEFAULT 100,
                                 excluded_from_search BOOLEAN NOT NULL DEFAULT FALSE, deleted_at TIMESTAMP);
             CREATE TABLE breaks (id INTEGER PRIMARY KEY, date DATE NOT NULL, start_time DATETIME NOT NULL,
                                  end_time DATETIME NOT NULL, duration INTEGER NOT NULL, reason TEXT,
                                  created_at DATETIME DEFAULT CURRENT_TIMESTAMP);

             -- Two days of history, in the bare wall-clock text the agent writes.
             INSERT INTO workdays (date, start, end) VALUES
                 ('2025-03-10', '2025-03-10 09:12:00', '2025-03-10 18:31:00'),
                 ('2025-03-11', '2025-03-11 09:05:00', NULL);

             INSERT INTO pauses (start, end, duration) VALUES
                 ('2025-03-10 13:02:00', '2025-03-10 14:05:00', 3780),
                 ('2025-03-11 11:00:00', NULL, NULL);

             INSERT INTO breaks (date, start_time, end_time, duration, reason) VALUES
                 ('2025-03-10', '2025-03-10 16:00:00', '2025-03-10 16:15:00', 900, 'coffee');

             -- Task 2 continues task 1's work; task 3 the employee deleted.
             INSERT INTO tasks (id, task_id, timestamp, name, comment, completeness, deleted_at) VALUES
                 (1, 0, '2025-03-10 17:50:00', 'Write the importer', 'first pass', 80, NULL),
                 (2, 1, '2025-03-11 17:55:00', 'Write the importer', NULL, 100, NULL),
                 (3, 0, '2025-03-10 18:00:00', 'Entered by mistake', NULL, 100, '2025-03-10 18:01:00');",
        )
        .expect("the fixture schema should apply");
}

fn offset(hours: i32) -> FixedOffset {
    FixedOffset::east_opt(hours * 3600).expect("a valid offset")
}

/// A scratch path for a fixture database, removed when the test ends.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kasl-import-{}-{name}.db", uuid::Uuid::new_v4().simple()));
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn reads_an_agent_database_the_way_the_agent_wrote_it() {
    let scratch = Scratch::new("read");
    agent_db(&scratch.0);

    let (days, summary) = import::read_agent_db(&scratch.0).expect("a kasl database should be readable");

    assert_eq!(summary.days, 2);
    assert_eq!(summary.pauses, 3, "two automatic pauses and one manual break");
    assert_eq!(summary.tasks, 2, "the deleted task is not among them");
    assert_eq!(summary.skipped_deleted_tasks, 1, "and it is counted, not silently dropped");

    let first = &days[0];
    assert_eq!(first.date.to_string(), "2025-03-10");
    assert_eq!(first.pauses.len(), 2);
    assert_eq!(first.tasks.len(), 1);

    // The manual break must arrive marked as one: in the agent it lives in a
    // different table, and losing the distinction would make every break look
    // like idle time on the dashboard.
    let manual = first.pauses.iter().find(|pause| pause.manual).expect("the break should be imported");
    assert_eq!(manual.reason.as_deref(), Some("coffee"));
    assert!(
        first.pauses.iter().any(|pause| !pause.manual),
        "and the automatic pause should still be automatic"
    );

    // An open day and an open pause survive as open.
    let second = &days[1];
    assert!(second.end.is_none(), "a day the employee never closed has no end");
    assert!(second.pauses[0].end.is_none());

    // The agent stores 0 for "this task belongs to itself".
    assert_eq!(first.tasks[0].agent_group_id, 1, "a group of 0 resolves to the task itself");
    assert_eq!(days[1].tasks[0].agent_group_id, 1, "and a real group is kept as it is");
}

#[tokio::test]
async fn a_year_of_local_history_lands_under_the_operators_offset() {
    let Some(server) = TestServer::start().await else { return };
    let scratch = Scratch::new("write");
    agent_db(&scratch.0);

    let user_id = import::resolve_user(&server.pool, "employee@example.test")
        .await
        .expect("the seeded user should resolve");

    let (days, _) = import::read_agent_db(&scratch.0).unwrap();
    let written = import::write_days(&server.pool, user_id, &days, offset(-3))
        .await
        .expect("the import should succeed");

    assert_eq!(written, 2);
    assert_eq!(server.count("workdays").await, 2);
    assert_eq!(server.count("pauses").await, 3);
    assert_eq!(server.count("tasks").await, 2);

    // The whole point of the offset argument: bare "09:12" becomes an absolute
    // instant, and it is the operator's answer that decided which one.
    let started_at: String = server
        .scalar("SELECT to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') FROM workdays WHERE date = '2025-03-10'")
        .await;
    assert_eq!(started_at, "2025-03-10 12:12", "09:12-03:00 is 12:12 UTC");

    // The employee's own date is kept as the agent recorded it, not re-derived.
    let date: String = server.scalar("SELECT to_char(date, 'YYYY-MM-DD') FROM workdays ORDER BY date LIMIT 1").await;
    assert_eq!(date, "2025-03-10");

    let manual: bool = server.scalar("SELECT manual FROM pauses WHERE reason = 'coffee'").await;
    assert!(manual, "the manual break must stay marked as the employee's own");
}

#[tokio::test]
async fn the_offset_chosen_is_the_offset_stored() {
    let Some(server) = TestServer::start().await else { return };
    let scratch = Scratch::new("offset");
    agent_db(&scratch.0);

    let user_id = import::resolve_user(&server.pool, "employee@example.test").await.unwrap();
    let (days, _) = import::read_agent_db(&scratch.0).unwrap();

    // The same file, imported as if the employee had been five hours east.
    import::write_days(&server.pool, user_id, &days, offset(5)).await.unwrap();

    let started_at: String = server
        .scalar("SELECT to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') FROM workdays WHERE date = '2025-03-10'")
        .await;
    assert_eq!(started_at, "2025-03-10 04:12", "09:12+05:00 is 04:12 UTC - a different moment entirely");
}

#[tokio::test]
async fn importing_twice_corrects_rather_than_doubles() {
    let Some(server) = TestServer::start().await else { return };
    let scratch = Scratch::new("twice");
    agent_db(&scratch.0);

    let user_id = import::resolve_user(&server.pool, "employee@example.test").await.unwrap();
    let (days, _) = import::read_agent_db(&scratch.0).unwrap();

    // An import that failed partway is re-run by hand; doing so must be safe,
    // and re-running with a corrected offset must fix what the first one got
    // wrong rather than leaving both versions.
    import::write_days(&server.pool, user_id, &days, offset(-3)).await.unwrap();
    import::write_days(&server.pool, user_id, &days, offset(5)).await.unwrap();

    assert_eq!(server.count("workdays").await, 2, "the same days must not become new ones");
    assert_eq!(server.count("pauses").await, 3, "nor the pauses pile up");
    assert_eq!(server.count("tasks").await, 2);

    let started_at: String = server
        .scalar("SELECT to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') FROM workdays WHERE date = '2025-03-10'")
        .await;
    assert_eq!(started_at, "2025-03-10 04:12", "the second import's offset must win");
}

#[test]
fn a_date_range_splits_a_history_that_crossed_time_zones() {
    let scratch = Scratch::new("range");
    agent_db(&scratch.0);
    let (days, _) = import::read_agent_db(&scratch.0).unwrap();

    let day = |text: &str| text.parse::<chrono::NaiveDate>().unwrap();

    // Both ends are inclusive: an operator splitting a year at a move date
    // writes the same date as the end of one run and the start of the next,
    // and neither day may fall between the two.
    let first = import::within(import::read_agent_db(&scratch.0).unwrap().0, None, Some(day("2025-03-10")));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].date, day("2025-03-10"), "the boundary day belongs to the earlier run");

    let second = import::within(import::read_agent_db(&scratch.0).unwrap().0, Some(day("2025-03-11")), None);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].date, day("2025-03-11"));

    // No bounds is everything, which is what an import without the arguments
    // must keep doing.
    assert_eq!(import::within(days, None, None).len(), 2);

    // A range covering nothing yields nothing rather than everything.
    let empty = import::within(import::read_agent_db(&scratch.0).unwrap().0, Some(day("2026-01-01")), None);
    assert!(empty.is_empty());
}

#[tokio::test]
async fn an_import_will_not_invent_the_person() {
    let Some(server) = TestServer::start().await else { return };

    // A typo in an email would otherwise file a year of someone's history under
    // an account nobody looks at, with nothing to notice it.
    let error = import::resolve_user(&server.pool, "employe@example.test")
        .await
        .expect_err("an unknown email must stop the import");
    assert!(error.to_string().contains("create the account"), "the message should say what to do: {error}");
}

#[test]
fn a_file_that_is_not_a_kasl_database_is_refused_clearly() {
    let scratch = Scratch::new("empty");
    rusqlite::Connection::open(&scratch.0)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (x INTEGER)")
        .unwrap();

    let error = import::read_agent_db(&scratch.0).expect_err("a database without kasl's tables must be refused");
    assert!(
        error.to_string().contains("kasl database"),
        "the message should name the likely mistake: {error}"
    );

    let missing = import::read_agent_db(std::path::Path::new("no-such-file.db")).expect_err("a missing file must be refused");
    assert!(missing.to_string().contains("no such file"), "{missing}");
}

/// What this can and cannot prove.
///
/// The importer opens the file with `SQLITE_OPEN_READ_ONLY`, and that flag is
/// real protection - but it is not observable from the outside while the code
/// does not write. SQLite opens a read-only *file* for writing without
/// complaint and only fails at the first write, and both modes touch the same
/// `-wal`/`-shm` files during a read; a mutation swapping the flag for a plain
/// open leaves every assertion below green. So this checks what is checkable -
/// the employee's file comes back byte-identical, and a file the operating
/// system forbids writing still imports - and the flag stands as the guard
/// against a future edit that does try to write.
#[test]
fn the_employees_file_survives_the_import_untouched() {
    let scratch = Scratch::new("readonly");
    agent_db(&scratch.0);

    let before = std::fs::metadata(&scratch.0).expect("the fixture exists").len();
    let modified_before = std::fs::metadata(&scratch.0).unwrap().modified().unwrap();

    import::read_agent_db(&scratch.0).expect("the import should read it");

    let after = std::fs::metadata(&scratch.0).unwrap().len();
    assert_eq!(before, after, "the file's size must not change");
    assert_eq!(
        modified_before,
        std::fs::metadata(&scratch.0).unwrap().modified().unwrap(),
        "nor its modification time"
    );

    // And it must still work when the file itself denies writing - an employee
    // hands over a copy from a backup or a locked-down share, and SQLite opened
    // for writing fails outright on such a file.
    let mut permissions = std::fs::metadata(&scratch.0).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&scratch.0, permissions).expect("the fixture should become read-only");

    let result = import::read_agent_db(&scratch.0);

    // Restore before asserting, or a failure here leaves an undeletable file.
    let mut permissions = std::fs::metadata(&scratch.0).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    let _ = std::fs::set_permissions(&scratch.0, permissions);

    let (days, _) = result.expect("a read-only file must still be importable");
    assert_eq!(days.len(), 2, "and yield the same history");
}

#[test]
fn an_older_agent_database_still_imports() {
    // Migrations 4 and 6 added `deleted_at` and `breaks`; a database from
    // before them is exactly what a long-serving employee has, and it must not
    // fail on the columns it lacks.
    let scratch = Scratch::new("old");
    let connection = rusqlite::Connection::open(&scratch.0).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE workdays (id INTEGER PRIMARY KEY, date DATE NOT NULL UNIQUE, start TIMESTAMP NOT NULL, end TIMESTAMP);
             CREATE TABLE pauses (id INTEGER PRIMARY KEY, start TIMESTAMP NOT NULL, end TIMESTAMP, duration INTEGER);
             CREATE TABLE tasks (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL DEFAULT 0, timestamp TIMESTAMP,
                                 name TEXT NOT NULL, comment TEXT, completeness INTEGER NOT NULL DEFAULT 100,
                                 excluded_from_search BOOLEAN NOT NULL DEFAULT FALSE);
             INSERT INTO workdays (date, start, end) VALUES ('2024-01-15', '2024-01-15 08:00:00', '2024-01-15 17:00:00');
             INSERT INTO tasks (id, task_id, timestamp, name, completeness) VALUES (1, 0, '2024-01-15 16:00:00', 'Old work', 100);",
        )
        .unwrap();

    let (days, summary) = import::read_agent_db(&scratch.0).expect("a pre-migration database should still import");
    assert_eq!(summary.days, 1);
    assert_eq!(summary.tasks, 1);
    assert_eq!(summary.skipped_deleted_tasks, 0, "a file without the column has nothing deleted to skip");
    assert_eq!(days[0].tasks[0].name, "Old work");
}

#[test]
fn a_row_with_unreadable_time_is_skipped_and_counted() {
    let scratch = Scratch::new("broken");
    let connection = rusqlite::Connection::open(&scratch.0).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE workdays (id INTEGER PRIMARY KEY, date DATE NOT NULL UNIQUE, start TIMESTAMP NOT NULL, end TIMESTAMP);
             CREATE TABLE pauses (id INTEGER PRIMARY KEY, start TIMESTAMP NOT NULL, end TIMESTAMP, duration INTEGER);
             CREATE TABLE tasks (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL DEFAULT 0, timestamp TIMESTAMP,
                                 name TEXT NOT NULL, comment TEXT, completeness INTEGER NOT NULL DEFAULT 100);
             INSERT INTO workdays (date, start, end) VALUES
                 ('2024-01-15', '2024-01-15 08:00:00', NULL),
                 ('2024-01-16', 'not a timestamp', NULL);",
        )
        .unwrap();

    // One bad row must not cost the employee the rest of their history.
    let (days, summary) = import::read_agent_db(&scratch.0).expect("a damaged row should not fail the import");
    assert_eq!(days.len(), 1, "the readable day survives");
    assert_eq!(summary.skipped_unreadable, 1, "and the damaged one is reported, not hidden");
}
