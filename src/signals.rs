//! What the dashboard should point at: `GET /api/v1/team/signals` and the
//! weekly trend behind `GET /api/v1/users/{id}/trend`.
//!
//! Every screen before this one answers a question the manager asked. This one
//! answers the question they did not know to ask - somebody's hours have been
//! sliding for three weeks, and no single view shows it, because the pattern
//! only exists across weeks and nobody scrolls back through weeks looking.
//!
//! The rules are in ADR 0016, and two of them shape every line here:
//!
//! * **A person is compared with themselves.** Never with a colleague, never
//!   with a norm - this server has none until the production calendar (v0.21),
//!   and a threshold invented before then would be this product asserting what
//!   a working day should be on somebody else's team.
//! * **A signal is a question, not a verdict.** Each one carries the figures it
//!   came from, so the screen can say "8.5 h → 5.0 h over three weeks" instead
//!   of a badge reading "problem". Falling hours are a holiday, a hospital, or
//!   a project that ended, and the server knows none of that.
//!
//! The arithmetic deliberately lives in Rust over weekly sums, not in SQL: the
//! statistics are the part most worth testing, and logic inside a query can
//! only be tested through a database.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin::{VISIBLE_USERS, require_manager_or_admin},
    app::AppState,
    error::ApiError,
    login::CurrentUser,
    model::UserRole,
};

/// How many complete weeks the trend and the signals look back over.
///
/// A quarter: long enough that a three-week slide has weeks behind it to be a
/// slide *from*, short enough that a person who changed roles in the spring is
/// not still being measured against who they were then.
pub const TREND_WEEKS: i64 = 12;

/// How many weeks each side of the comparison a decline is measured over.
///
/// Three, not two. Two weeks is one bad week next to one ordinary one, and a
/// dashboard that fired on that would flag everybody who took a Friday off -
/// which teaches people to ignore the column (the lesson `unknown` taught in
/// ADR 0014). Six weeks of history are needed before the question can be
/// asked at all.
const DECLINING_WEEKS: usize = 3;

/// How far the recent level must sit below the earlier one to be a decline,
/// as a fraction of the earlier level.
///
/// Fifteen per cent: a slide worth a manager's attention, and above the noise
/// of one short week inside a three-week median. Taken from the demo's fading
/// person, whose real shape comes to about nineteen per cent - the threshold
/// has to catch that without firing on ordinary variation.
const DECLINING_FRACTION: f64 = 0.15;

/// How far from a person's own median a week must fall to be called unusual,
/// as a fraction of that median.
///
/// Forty per cent in either direction: a four-day week is about twenty per
/// cent down and is nobody's business, while half a week or half again is
/// something a manager would want to have noticed themselves.
const UNUSUAL_FRACTION: f64 = 0.4;

/// The fewest weeks with any hours before a person's median means anything.
///
/// Below this there is no "usual for them" to compare against, and a signal
/// derived from two weeks of history would be an opinion about a new hire.
const MIN_WEEKS_FOR_MEDIAN: usize = 4;

/// What kind of thing the server noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// The recent weeks sit well below the weeks before them.
    Declining,
    /// Nothing recorded for longer than this person's own usual gap.
    NoData,
    /// The last complete week is far from this person's own median, either way.
    UnusualWeek,
}

/// One thing worth looking at, about one person.
///
/// The figures travel with the signal so the screen states what happened
/// rather than what it means: the server saw hours fall from 8.5 to 5.0, and
/// what that is about is between the manager and the person.
#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub user_id: Uuid,
    pub display_name: String,
    pub department: Option<String>,
    pub kind: SignalKind,
    /// Weeks on each side of the comparison, for `declining`.
    pub weeks: Option<i64>,
    /// The earlier level, in seconds of a typical week - the median of the
    /// weeks before the recent ones. `declining` only.
    pub from_seconds: Option<i64>,
    /// The recent level, on the same terms. `declining` and `unusual_week`.
    pub to_seconds: Option<i64>,
    /// This person's own median week, in seconds. `unusual_week` only - it is
    /// the thing being compared against, and a screen that showed the deviation
    /// without it would be quoting a percentage of nothing.
    pub median_seconds: Option<i64>,
    /// Days since the last recorded day. `no_data` only.
    pub days_quiet: Option<i64>,
}

/// The team's signals, most worth looking at first.
#[derive(Debug, Serialize)]
pub struct Signals {
    /// The window the signals were computed over.
    pub from: NaiveDate,
    /// The last day of the last **complete** week. The current week is never
    /// included: a Tuesday is not a short week, but it looks like one to
    /// arithmetic, and `declining` would fire on the whole team every Monday.
    pub to: NaiveDate,
    pub signals: Vec<Signal>,
    /// People examined. `0 of 12` is a different message from "nothing wrong",
    /// and a screen that cannot tell them apart says the reassuring one.
    pub people: i64,
}

/// One person's week on the trend chart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendWeek {
    /// The Monday the week starts on.
    pub week_start: NaiveDate,
    pub worked_seconds: i64,
    /// Days with a workday in that week. Zero says the silence is real rather
    /// than a week of very short days.
    pub days_recorded: i64,
}

/// The weekly trend for one person.
#[derive(Debug, Serialize)]
pub struct Trend {
    pub user_id: Uuid,
    /// Every complete week in the window, including the empty ones - a gap
    /// drawn as a gap is the point of the chart, and dropping empty weeks
    /// would close it up and hide the absence.
    pub weeks: Vec<TrendWeek>,
    /// This person's median week over the window, in seconds; `null` when
    /// there is too little history for a median to mean anything.
    pub median_seconds: Option<i64>,
    /// What the server noticed about this person, if anything.
    pub signals: Vec<Signal>,
}

/// Answers the signals for everyone the reader may see.
pub async fn team(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let (from, to) = window(Utc::now().date_naive());
    let rows = weekly_totals(&state.pool, &user, None, from, to).await?;
    let people = rows.len() as i64;

    let mut signals: Vec<Signal> = rows.iter().flat_map(|person| person.signals(to)).collect();
    // Worst first: a list a manager reads top-down should start with silence,
    // then the declines, rather than with whoever sorts first by name.
    signals.sort_by_key(|signal| (severity(signal.kind), -signal.weeks.unwrap_or(0), -signal.days_quiet.unwrap_or(0)));

    Ok(Json(Signals { from, to, signals, people }))
}

/// Answers one person's weekly trend, to someone allowed to see them.
pub async fn user_trend(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Query(_): Query<TrendQuery>,
) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let (from, to) = window(Utc::now().date_naive());
    let rows = weekly_totals(&state.pool, &user, Some(target), from, to).await?;

    // Not "no such user": a manager probing ids must not be able to tell an
    // employee in another department from one who does not exist - the same
    // rule the drill-down follows in `team`.
    let Some(person) = rows.into_iter().next() else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user"));
    };

    let signals = person.signals(to);
    let weeks = person.filled_weeks(from, to);
    let median_seconds = person.median();

    Ok(Json(Trend {
        user_id: person.user_id,
        weeks,
        median_seconds,
        signals,
    }))
}

/// Nothing yet - the window is fixed at [`TREND_WEEKS`]. Present so the route
/// can grow one without becoming a different path.
#[derive(Debug, Deserialize)]
pub struct TrendQuery {}

/// The window the signals and the trend are computed over: complete weeks only.
///
/// Ends on the Sunday before the current week, so a partial week never enters
/// the arithmetic. Separate from the handlers so "which weeks count" can be
/// tested against a fixed date rather than against whatever day CI runs on.
pub fn window(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let this_monday = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let last_sunday = this_monday - chrono::Duration::days(1);
    let first_monday = this_monday - chrono::Duration::weeks(TREND_WEEKS);
    (first_monday, last_sunday)
}

/// Which signals a screen shows first. Lower sorts earlier.
fn severity(kind: SignalKind) -> u8 {
    match kind {
        // Silence outranks a slide: an agent that stopped reporting means the
        // other numbers about that person are not to be trusted either.
        SignalKind::NoData => 0,
        SignalKind::Declining => 1,
        SignalKind::UnusualWeek => 2,
    }
}

/// One person with their weekly sums, as the database grouped them.
#[derive(Debug)]
struct Person {
    user_id: Uuid,
    display_name: String,
    department: Option<String>,
    /// Only the weeks with days in them, ascending. Absent weeks are absent
    /// data, filled in by [`Person::filled_weeks`] where a chart needs them.
    weeks: Vec<TrendWeek>,
    /// The last date with a workday, at any time - not only inside the window,
    /// so "quiet since June" is measured from June rather than from the edge of
    /// the chart.
    last_day: Option<NaiveDate>,
}

impl Person {
    /// This person's median week, over the weeks they actually worked.
    ///
    /// The median rather than the mean: one crunch week drags a mean far
    /// enough to hide a real decline behind it, and one week off drags it the
    /// other way. `None` when there is too little history for "usual for them"
    /// to mean anything.
    fn median(&self) -> Option<i64> {
        let mut worked: Vec<i64> = self.weeks.iter().map(|week| week.worked_seconds).filter(|seconds| *seconds > 0).collect();
        if worked.len() < MIN_WEEKS_FOR_MEDIAN {
            return None;
        }
        worked.sort_unstable();
        Some(median_of_sorted(&worked))
    }

    /// Every week in the window, including the ones with nothing in them.
    fn filled_weeks(&self, from: NaiveDate, to: NaiveDate) -> Vec<TrendWeek> {
        let mut weeks = Vec::new();
        let mut monday = from;
        while monday <= to {
            let recorded = self.weeks.iter().find(|week| week.week_start == monday);
            weeks.push(TrendWeek {
                week_start: monday,
                worked_seconds: recorded.map_or(0, |week| week.worked_seconds),
                days_recorded: recorded.map_or(0, |week| week.days_recorded),
            });
            monday += chrono::Duration::weeks(1);
        }
        weeks
    }

    /// What the server noticed about this person.
    fn signals(&self, to: NaiveDate) -> Vec<Signal> {
        let mut found = Vec::new();

        if let Some((weeks, from_seconds, to_seconds)) = self.declining() {
            found.push(self.signal(SignalKind::Declining, |signal| {
                signal.weeks = Some(weeks as i64);
                signal.from_seconds = Some(from_seconds);
                signal.to_seconds = Some(to_seconds);
            }));
        }

        if let Some(days_quiet) = self.quiet_for(to) {
            found.push(self.signal(SignalKind::NoData, |signal| signal.days_quiet = Some(days_quiet)));
        }

        if let Some((week_seconds, median)) = self.unusual_week(to) {
            found.push(self.signal(SignalKind::UnusualWeek, |signal| {
                signal.to_seconds = Some(week_seconds);
                signal.median_seconds = Some(median);
            }));
        }

        found
    }

    /// A signal about this person, with the figures filled in by the caller.
    fn signal(&self, kind: SignalKind, fill: impl FnOnce(&mut Signal)) -> Signal {
        let mut signal = Signal {
            user_id: self.user_id,
            display_name: self.display_name.clone(),
            department: self.department.clone(),
            kind,
            weeks: None,
            from_seconds: None,
            to_seconds: None,
            median_seconds: None,
            days_quiet: None,
        };
        fill(&mut signal);
        signal
    }

    /// Whether the recent level of work sits well below the level before it,
    /// and between which figures - as `(weeks compared, before, now)`.
    ///
    /// **Levels, not steps.** An earlier version asked for three weeks each
    /// lower than the last, and a live run against the demo showed why that is
    /// the wrong question: a genuinely fading person went 33 → 24.8 → 27 →
    /// 20.9 → 23.1 → 22.1, which is an unmistakable slide and never three
    /// falls in a row. One ordinary week in the middle resets a run, so the
    /// strict version stays silent on exactly the case the milestone exists
    /// for. Comparing the median of the last three weeks with the median of
    /// the three before them sees the same data as a nineteen per cent drop.
    ///
    /// Medians on both sides for the usual reason: one crunch week either side
    /// would otherwise decide the answer on its own.
    ///
    /// Computed over the weeks that actually have hours. A week the database
    /// never grouped is simply absent; the zero that does arrive is a week
    /// whose days were all still open, and skipping it keeps a slide visible
    /// *through* it rather than letting a late-filed week look like a crash.
    fn declining(&self) -> Option<(usize, i64, i64)> {
        let worked: Vec<i64> = self.weeks.iter().map(|week| week.worked_seconds).filter(|seconds| *seconds > 0).collect();

        // Two windows' worth of weeks, or there is no "before" to fall from.
        if worked.len() < DECLINING_WEEKS * 2 {
            return None;
        }

        let recent = median_of(&worked[worked.len() - DECLINING_WEEKS..]);
        let before = median_of(&worked[worked.len() - DECLINING_WEEKS * 2..worked.len() - DECLINING_WEEKS]);

        if before <= 0 {
            return None;
        }

        let drop = (before - recent) as f64 / before as f64;
        (drop >= DECLINING_FRACTION).then_some((DECLINING_WEEKS, before, recent))
    }

    /// Days since the last recorded day, when that is longer than this person's
    /// own rhythm allows.
    ///
    /// Their own rhythm, not a fixed number: somebody who reports every day and
    /// somebody who files a week at a time are both normal, and one threshold
    /// for the two would either miss the first or nag the second.
    fn quiet_for(&self, to: NaiveDate) -> Option<i64> {
        // Never reported at all is a different fact, and the dashboard already
        // has better words for it - "never reported", next to an agent count.
        let last_day = self.last_day?;
        let days = (to - last_day).num_days();

        // The window closed on a Sunday, so a person who worked to the end of
        // the last week is already one or two days "quiet". Ten days means a
        // week and a half with nothing at all, which no ordinary rhythm covers.
        (days >= 10).then_some(days)
    }

    /// The last complete week, when it is far from this person's own median.
    fn unusual_week(&self, to: NaiveDate) -> Option<(i64, i64)> {
        let median = self.median()?;

        // The last week of the *window*, not the last week with data in it.
        // `weeks` holds only the weeks somebody worked, so its final entry can
        // be a fortnight old - and calling that "last week" would describe a
        // week nobody is thinking about. A live run found this: somebody who
        // stopped reporting was told their last week was short, when the week
        // in question had ended thirteen days earlier.
        let last_monday = to - chrono::Duration::days(6);
        let last = self.weeks.iter().find(|week| week.week_start == last_monday)?;

        // A week with nothing in it is silence, and `no_data` is the signal
        // for that. Calling it "unusual" too would report one fact twice.
        if last.worked_seconds == 0 {
            return None;
        }

        let deviation = (last.worked_seconds - median).abs() as f64 / median as f64;
        (deviation >= UNUSUAL_FRACTION).then_some((last.worked_seconds, median))
    }
}

/// The median of a slice in any order. Empty answers zero, which every caller
/// guards against before it can mean anything.
fn median_of(values: &[i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    median_of_sorted(&sorted)
}

/// The middle of a sorted, non-empty slice; the mean of the two middles when
/// the count is even.
fn median_of_sorted(sorted: &[i64]) -> i64 {
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2
    }
}

/// A row of the query: one person's one week.
#[derive(Debug, sqlx::FromRow)]
struct WeekRow {
    user_id: Uuid,
    display_name: String,
    department: Option<String>,
    week_start: Option<NaiveDate>,
    worked_seconds: Option<i64>,
    days_recorded: Option<i64>,
    last_day: Option<NaiveDate>,
}

/// Loads weekly sums for everyone the reader may see, or for one person.
///
/// Grouped by week in the database and reasoned about in Rust: the grouping is
/// what a database is for, and the statistics are what a test can only reach
/// outside one.
async fn weekly_totals(pool: &PgPool, reader: &CurrentUser, only: Option<Uuid>, from: NaiveDate, to: NaiveDate) -> Result<Vec<Person>, ApiError> {
    // `AssertSqlSafe` because the only interpolation is `VISIBLE_USERS`, a
    // constant in `admin`, and a fixed clause; every value is bound below.
    let one_person = if only.is_some() { "AND u.id = $5" } else { "" };

    let rows: Vec<WeekRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT u.id AS user_id, u.display_name, d.name AS department,
                w.week_start, w.worked_seconds, w.days_recorded,
                (SELECT max(all_days.date) FROM workdays all_days WHERE all_days.user_id = u.id) AS last_day
         FROM users u
         LEFT JOIN departments d ON d.id = u.department_id
         LEFT JOIN LATERAL (
             SELECT (date_trunc('week', wd.date))::date AS week_start,
                    count(*)::bigint AS days_recorded,
                    coalesce(sum(
                        CASE WHEN wd.ended_at IS NULL THEN 0
                             ELSE greatest(extract(epoch FROM (wd.ended_at - wd.started_at))::bigint - paused.seconds, 0)
                        END
                    ), 0)::bigint AS worked_seconds
             FROM workdays wd
             CROSS JOIN LATERAL (
                 -- Stored pauses where they exist; the day's own totals where a
                 -- narrower policy summarized them away (ADR 0011). One or the
                 -- other, never both, so an hour cannot be counted twice.
                 SELECT CASE
                     WHEN EXISTS (SELECT 1 FROM pauses p WHERE p.workday_id = wd.id)
                     THEN (SELECT coalesce(sum(p.duration_seconds), 0)::bigint FROM pauses p WHERE p.workday_id = wd.id)
                     ELSE coalesce(wd.paused_seconds, 0)::bigint
                 END AS seconds
             ) AS paused
             WHERE wd.user_id = u.id AND wd.date BETWEEN $3 AND $4
             GROUP BY 1
         ) AS w ON true
         WHERE u.active AND {VISIBLE_USERS} {one_person}
         ORDER BY u.display_name, u.email, w.week_start"
    )))
    .bind(reader.role == UserRole::Admin)
    .bind(reader.user_id)
    .bind(from)
    .bind(to)
    // Bound unconditionally: sqlx counts placeholders in the string it was
    // given, and an unused bind is cheaper than two nearly identical queries.
    .bind(only.unwrap_or_else(Uuid::nil))
    .fetch_all(pool)
    .await?;

    Ok(into_people(rows))
}

/// Folds the flat rows into one entry per person.
///
/// Relies on the query's `ORDER BY` keeping each person's weeks together, so
/// this is a single pass and the order the database chose is the order the
/// screen draws.
fn into_people(rows: Vec<WeekRow>) -> Vec<Person> {
    let mut people: Vec<Person> = Vec::new();

    for row in rows {
        if people.last().map(|person| person.user_id) != Some(row.user_id) {
            people.push(Person {
                user_id: row.user_id,
                display_name: row.display_name,
                department: row.department,
                weeks: Vec::new(),
                last_day: row.last_day,
            });
        }

        let person = people.last_mut().expect("a person was just pushed");

        // No week means the outer join found nothing: the person is the answer,
        // the week is not.
        let Some(week_start) = row.week_start else { continue };

        person.weeks.push(TrendWeek {
            week_start,
            worked_seconds: row.worked_seconds.unwrap_or(0),
            days_recorded: row.days_recorded.unwrap_or(0),
        });
    }

    people
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    /// The Monday twelve-week window ending 2026-08-30, as the tests use it.
    const LAST_MONDAY: &str = "2026-08-24";

    /// A person whose weeks carry the given hours, oldest first, the last of
    /// them being the week starting [`LAST_MONDAY`].
    ///
    /// A zero is a week nobody recorded, which the query answers by not
    /// answering it at all - so it is dropped here too, exactly as the database
    /// would drop it. For the *other* kind of zero see [`person_with_open`].
    fn person_with(hours: &[f64]) -> Person {
        build(hours, false)
    }

    /// The same, but a zero is a week whose days were all still open.
    ///
    /// This one the database really does emit: `GROUP BY` counts the days, and
    /// an unfinished day contributes no seconds, so the row arrives with
    /// `days_recorded > 0` and `worked_seconds = 0`. It is the only way a zero
    /// reaches the statistics, and the case where reading it as "worked
    /// nothing" would invent a crash out of a week that is simply not over.
    fn person_with_open(hours: &[f64]) -> Person {
        build(hours, true)
    }

    fn build(hours: &[f64], keep_zero_weeks: bool) -> Person {
        let last = date(LAST_MONDAY);
        let count = hours.len();
        let weeks: Vec<TrendWeek> = hours
            .iter()
            .enumerate()
            .map(|(index, hours)| TrendWeek {
                week_start: last - chrono::Duration::weeks((count - 1 - index) as i64),
                worked_seconds: (hours * 3600.0) as i64,
                days_recorded: if *hours > 0.0 || keep_zero_weeks { 5 } else { 0 },
            })
            .filter(|week| week.days_recorded > 0)
            .collect();

        Person {
            user_id: Uuid::new_v4(),
            display_name: "Test".into(),
            department: None,
            weeks,
            // Friday of the last week: someone who worked right up to the end
            // of the window, so `no_data` stays quiet unless a test says so.
            last_day: Some(last + chrono::Duration::days(4)),
        }
    }

    /// The Sunday the window closes on, for the tests that need it.
    fn window_end() -> NaiveDate {
        date(LAST_MONDAY) + chrono::Duration::days(6)
    }

    #[test]
    fn the_window_ends_at_the_last_complete_week() {
        // A Wednesday: the window must stop at the Sunday before this Monday,
        // never reaching into the half-lived current week.
        let (from, to) = window(date("2026-09-02"));
        assert_eq!(to, date("2026-08-30"), "the Sunday before this week");
        assert_eq!(from, date("2026-06-08"), "twelve complete weeks back");
        assert_eq!((to - from).num_days() + 1, TREND_WEEKS * 7);
    }

    #[test]
    fn a_monday_does_not_count_its_own_week() {
        // The edge that would otherwise fire `declining` across the team every
        // Monday morning: today's week has one day in it and is not a week.
        let (_, to) = window(date("2026-08-31"));
        assert_eq!(to, date("2026-08-30"));
    }

    #[test]
    fn a_sunday_belongs_to_the_week_that_just_ended() {
        // Sunday is the last day of its own week, so that week is complete.
        let (_, to) = window(date("2026-08-30"));
        assert_eq!(to, date("2026-08-23"), "the week containing Sunday is still the current one");
    }

    #[test]
    fn the_median_is_the_middle_not_the_mean() {
        // One crunch week is exactly what would hide a decline behind a mean.
        assert_eq!(median_of_sorted(&[1, 2, 3, 4, 40]), 3);
        assert_eq!(median_of_sorted(&[2, 4]), 3);
        assert_eq!(median_of_sorted(&[7]), 7);
    }

    #[test]
    fn a_decline_compares_levels_rather_than_counting_steps() {
        // Six weeks: a settled level, then a clearly lower one. The figures
        // reported are the two medians, so the screen can say "40 h a week
        // down to 30 h" without quoting a week nobody worked.
        let slid = person_with(&[40.0, 40.0, 40.0, 30.0, 30.0, 30.0]);
        let (weeks, before, now) = slid.declining().expect("a quarter off the level is a decline");
        assert_eq!(weeks, DECLINING_WEEKS);
        assert_eq!(before, 40 * 3600);
        assert_eq!(now, 30 * 3600);
    }

    #[test]
    fn an_uneven_slide_is_still_a_slide() {
        // The shape that made this a levels comparison in the first place: the
        // demo's fading person, as the seed really produces him. Never three
        // falls in a row - one ordinary week in the middle resets a run - and
        // unmistakably a decline to anyone looking at the chart.
        let lukas = person_with(&[35.0, 33.0, 24.8, 27.0, 20.9, 23.1, 22.1]);
        let (_, before, now) = lukas.declining().expect("an uneven slide must not go unreported");
        assert!(before > now, "the direction has to survive the medians: {before} -> {now}");
    }

    #[test]
    fn ordinary_variation_is_not_a_decline() {
        // Weeks wobble. A signal that fired on this would fire on everybody,
        // and a list everybody is on is not read.
        let wobbly = person_with(&[40.0, 37.0, 41.0, 39.0, 38.0, 40.0]);
        assert_eq!(wobbly.declining(), None);
    }

    #[test]
    fn a_recovery_is_not_reported_as_a_decline() {
        // Down and then back to the old level: nothing to point at now, which
        // is the question this screen answers.
        let recovered = person_with(&[40.0, 30.0, 30.0, 40.0, 40.0, 41.0]);
        assert_eq!(recovered.declining(), None);
    }

    #[test]
    fn too_little_history_cannot_show_a_decline() {
        // Five weeks is not two three-week windows, so there is no "before" to
        // have fallen from. Answering anything here would be an opinion about
        // somebody who just arrived.
        let short = person_with(&[40.0, 40.0, 40.0, 20.0, 20.0]);
        assert_eq!(short.declining(), None);
    }

    #[test]
    fn a_missing_week_is_a_gap_not_a_crash() {
        // A fortnight off in the middle of a flat stretch. Reading the absent
        // weeks as zeroes would drag the recent median to nothing and report a
        // holiday as a collapse.
        let holiday = person_with(&[40.0, 40.0, 40.0, 0.0, 0.0, 40.0, 40.0, 40.0]);
        assert_eq!(holiday.declining(), None, "a gap is missing data, not falling hours");
    }

    #[test]
    fn a_week_of_days_still_open_is_not_a_week_of_no_work() {
        // The one zero that actually reaches the statistics: `GROUP BY` counts
        // the days, but an unfinished day contributes no seconds, so the week
        // arrives as `days_recorded = 5, worked_seconds = 0`. Reading it as
        // "worked nothing" manufactures a collapse out of a week that is
        // simply not filed yet - and, on the way back up, a recovery.
        let filing_late = person_with_open(&[40.0, 40.0, 40.0, 40.0, 0.0]);
        assert_eq!(filing_late.declining(), None, "an unfiled week is not a decline");

        // And the case that decides the filter: a week of unfiled days among
        // the recent ones. Counted as a zero it drags the recent median to the
        // floor and reports a collapse that is really one week filed late -
        // a signal about the agent, dressed up as one about the person.
        let interrupted = person_with_open(&[40.0, 40.0, 40.0, 39.0, 0.0, 41.0]);
        assert_eq!(interrupted.declining(), None, "an unfiled week must not be read as a week of no work");

        // And it must not drag the median down either, or every ordinary week
        // after it would start looking unusually long.
        assert_eq!(filing_late.median(), Some(40 * 3600));
    }

    #[test]
    fn silence_is_measured_from_the_last_real_day() {
        let mut quiet = person_with(&[40.0, 40.0, 40.0, 40.0]);
        // Stopped reporting a fortnight before the window closed.
        quiet.last_day = Some(window_end() - chrono::Duration::days(14));
        assert_eq!(quiet.quiet_for(window_end()), Some(14));

        // Someone who worked to the end of the window is not "quiet" just
        // because the window closes on a Sunday.
        let working = person_with(&[40.0, 40.0, 40.0, 40.0]);
        assert_eq!(working.quiet_for(window_end()), None);
    }

    #[test]
    fn someone_who_never_reported_gets_no_silence_signal() {
        // A different fact, and the dashboard already has better words for it:
        // "never reported", next to an agent count. Saying "no data for 84
        // days" about somebody who never had any would be arithmetic dressed
        // up as an observation.
        let mut never = person_with(&[]);
        never.last_day = None;
        assert_eq!(never.quiet_for(window_end()), None);
    }

    #[test]
    fn an_unusual_week_is_unusual_in_either_direction() {
        // Both ways on purpose: half the usual hours and half again are each
        // worth a look, and flagging only the low one would make the signal an
        // accusation rather than a question.
        let low = person_with(&[40.0, 40.0, 40.0, 40.0, 20.0]);
        let (week, median) = low.unusual_week(window_end()).expect("half the usual week is unusual");
        assert_eq!(week, 20 * 3600);
        assert_eq!(median, 40 * 3600);

        let high = person_with(&[40.0, 40.0, 40.0, 40.0, 60.0]);
        assert!(high.unusual_week(window_end()).is_some(), "a week half again as long is unusual too");

        // A four-day week is about a fifth down and is nobody's business.
        let ordinary = person_with(&[40.0, 40.0, 40.0, 40.0, 32.0]);
        assert_eq!(ordinary.unusual_week(window_end()), None);
    }

    #[test]
    fn too_little_history_means_no_median_and_no_signal() {
        // Three weeks of history is not a person's "usual", and a signal drawn
        // from it would be an opinion about a new hire.
        let new_hire = person_with(&[40.0, 40.0, 10.0]);
        assert_eq!(new_hire.median(), None);
        assert_eq!(new_hire.unusual_week(window_end()), None, "no median, no comparison");
    }

    #[test]
    fn an_empty_last_week_is_reported_as_silence_only_once() {
        // Zero hours in the final week is silence, and `no_data` is the signal
        // for that. Calling it "unusual" as well would state one fact twice
        // and put the same person on the list under two headings.
        let mut stopped = person_with(&[40.0, 40.0, 40.0, 40.0, 0.0]);
        stopped.last_day = Some(window_end() - chrono::Duration::days(11));

        assert_eq!(stopped.unusual_week(window_end()), None);
        let kinds: Vec<SignalKind> = stopped.signals(window_end()).into_iter().map(|signal| signal.kind).collect();
        assert_eq!(kinds, vec![SignalKind::NoData], "one fact, one signal");
    }

    #[test]
    fn last_week_means_the_last_week_of_the_window() {
        // Found by a live run, not by any of the tests above. `weeks` holds
        // only the weeks somebody worked, so its final entry can be a
        // fortnight old - and describing that as "last week" tells a manager
        // about a week nobody is thinking about, alongside a `no_data` signal
        // that says the person has been quiet since before it.
        // A short week, and then nothing at all - the shape the demo's silent
        // person really has. `person_with` drops the trailing zero the way the
        // database drops a week nobody worked, so the newest week on record is
        // the short one while the window has moved on past it.
        let stopped = person_with(&[40.0, 40.0, 40.0, 40.0, 7.0, 0.0]);
        assert_eq!(
            stopped.weeks.last().map(|week| week.worked_seconds),
            Some(7 * 3600),
            "the fixture must leave the short week as the newest one on record,              or this test proves nothing"
        );
        assert!(
            stopped.weeks.iter().all(|week| week.week_start < date(LAST_MONDAY)),
            "and the window's own last week must be missing from the data"
        );

        assert_eq!(
            stopped.unusual_week(window_end()),
            None,
            "a week that is not last week must not be reported as last week"
        );
    }

    #[test]
    fn a_steady_person_produces_nothing() {
        // The case that must stay silent, or the list stops being read.
        let steady = person_with(&[40.0, 39.0, 41.0, 40.0, 40.5]);
        assert!(steady.signals(window_end()).is_empty(), "a steady person is not news");
    }

    #[test]
    fn the_fading_person_from_the_demo_is_caught() {
        // The shape the demo seeds on purpose: a full week at the start,
        // five-hour days by the end. This milestone exists to point at it.
        let fading = person_with(&[42.5, 40.0, 37.5, 32.5, 30.0, 25.0]);
        let signals = fading.signals(window_end());

        let declining = signals
            .iter()
            .find(|signal| signal.kind == SignalKind::Declining)
            .expect("the fading person should be flagged");
        assert!(declining.weeks.unwrap_or(0) >= 3, "{declining:?}");
        assert!(
            declining.from_seconds > declining.to_seconds,
            "the figures must show the direction: {declining:?}"
        );
    }

    #[test]
    fn a_trend_keeps_its_empty_weeks() {
        // A gap drawn as a gap is the point of the chart. Dropping empty weeks
        // would close the hole up and make an absence look like continuity.
        let person = person_with(&[40.0, 0.0, 40.0]);
        let weeks = person.filled_weeks(date(LAST_MONDAY) - chrono::Duration::weeks(2), date(LAST_MONDAY));

        assert_eq!(weeks.len(), 3, "every week in the window, not only the worked ones");
        assert_eq!(weeks[1].worked_seconds, 0);
        assert_eq!(weeks[1].days_recorded, 0, "zero days says the silence is real");
    }

    #[test]
    fn signals_are_ordered_worst_first() {
        // A manager reads top-down, so silence outranks a slide: an agent that
        // stopped reporting means the other numbers about that person are not
        // to be trusted either.
        let mut kinds = [SignalKind::UnusualWeek, SignalKind::Declining, SignalKind::NoData];
        kinds.sort_by_key(|kind| severity(*kind));
        assert_eq!(kinds, [SignalKind::NoData, SignalKind::Declining, SignalKind::UnusualWeek]);
    }
}
