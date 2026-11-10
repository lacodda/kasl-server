//! Overtime rules: what an hour beyond the ordinary pays (ADR 0028).
//!
//! A version of the terms carries up to three rules, each optional:
//!
//! * **daily** - past a threshold of hours worked on a date, every further
//!   hour pays a multiplier, and optionally a higher one some hours later;
//! * **weekly** - the same over a Monday-to-Sunday week, counting only the
//!   hours no other rule already paid more for;
//! * **rest days** - every hour of a date the calendar asks nothing of pays a
//!   multiplier.
//!
//! A threshold is either fixed hours or the span's own norm. California's day
//! is "1.5 after 8 hours, 2 after 12" whoever works it; Russia's is "1.5 for
//! the first two hours past the norm, 2 after", and the norm of a short eve or
//! a half-time contract is not eight hours. Both are one shape: a threshold, a
//! multiplier, and an optional second step.
//!
//! The rules are columns of the version's row rather than rows of their own,
//! so they are exactly as fixed as its rate: nothing can add a rule to a
//! version after money was computed from it. Whatever the API accepts, the
//! database checks again.

use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};
use sqlx::{Row, postgres::PgRow};

use crate::error::ApiError;

/// The most decimal places a multiplier carries, as the columns hold it.
const MULTIPLIER_SCALE: u32 = 4;

/// The largest multiplier accepted. Laws stop at two or three; ten is a bound
/// on typing, not on policy.
const MULTIPLIER_MAX: i64 = 10;

/// The most decimal places an hour figure carries: hundredths, so a threshold
/// can be 7.5 or 7.75 hours and still be a whole number of seconds.
const HOURS_SCALE: u32 = 2;

/// How long a span is, and so how far a fixed ladder may reach into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Day,
    Week,
}

impl Span {
    fn max_hours(self) -> Decimal {
        match self {
            Self::Day => Decimal::from(24),
            Self::Week => Decimal::from(168),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Day => "daily",
            Self::Week => "weekly",
        }
    }
}

/// Where a ladder's first step starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    /// Past the span's norm: the date's, as the calendar and the person's share
    /// of a full day ask for it, or the week's, less any leave.
    Norm,
    /// Past this many hours worked in the span.
    Hours(Decimal),
}

impl Threshold {
    /// The threshold in seconds, given the span's norm in seconds.
    pub fn seconds(self, norm_seconds: i64) -> i64 {
        match self {
            Self::Norm => norm_seconds,
            Self::Hours(hours) => hours_to_seconds(hours),
        }
    }
}

/// The second step of a ladder: some hours past where the first one starts,
/// a higher multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Hours past the start of the first step.
    pub after: Decimal,
    pub multiplier: Decimal,
}

/// One rule: a threshold, what the hours past it pay, and an optional second
/// step further on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ladder {
    pub after: Threshold,
    pub multiplier: Decimal,
    pub then: Option<Step>,
}

impl Ladder {
    /// Where each step starts, in seconds into the span, with the multiplier
    /// it pays: one entry, or two in ascending order.
    pub fn steps(&self, norm_seconds: i64) -> Vec<(i64, Decimal)> {
        let first = self.after.seconds(norm_seconds);
        let mut steps = vec![(first, self.multiplier)];
        if let Some(then) = self.then {
            steps.push((first + hours_to_seconds(then.after), then.multiplier));
        }
        steps
    }
}

/// The overtime rules of one version of the terms. All three absent is a
/// version without overtime, which is every version recorded before 0.32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overtime {
    pub daily: Option<Ladder>,
    pub weekly: Option<Ladder>,
    /// What every hour of a rest day pays: a weekend the calendar did not move
    /// into the week, or a holiday.
    pub rest_days: Option<Decimal>,
}

/// Whole seconds in a figure of hours. Hours carry at most two decimal places,
/// so the product is always whole; rounding only guards the type.
pub fn hours_to_seconds(hours: Decimal) -> i64 {
    (hours * Decimal::from(3600)).round().to_i64().unwrap_or(i64::MAX)
}

// The wire ------------------------------------------------------------------

/// The rules as the API writes and reads them. Every figure a string, like the
/// rate: a JSON number is a float.
///
/// ```json
/// { "daily": { "after": "norm", "multiplier": "1.5", "then": { "after": "2", "multiplier": "2" } },
///   "weekly": { "after": "40", "multiplier": "1.5", "then": null },
///   "rest_days": "2" }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OvertimeWire {
    #[serde(default)]
    pub daily: Option<LadderWire>,
    #[serde(default)]
    pub weekly: Option<LadderWire>,
    /// The multiplier of every hour on a rest day.
    #[serde(default)]
    pub rest_days: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LadderWire {
    /// `"norm"`, or hours worked in the span: `"8"`, `"40"`.
    pub after: String,
    pub multiplier: String,
    #[serde(default)]
    pub then: Option<StepWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StepWire {
    /// Hours past the start of the first step.
    pub after: String,
    pub multiplier: String,
}

/// The word that sets a threshold at the span's norm.
const NORM: &str = "norm";

impl Overtime {
    /// Whether the version pays anything beyond the ordinary.
    pub fn is_none(&self) -> bool {
        self.daily.is_none() && self.weekly.is_none() && self.rest_days.is_none()
    }

    /// Reads the rules as sent, refusing any the columns would change or a
    /// person could not mean.
    pub fn parse(wire: &OvertimeWire) -> Result<Self, ApiError> {
        Ok(Self {
            daily: wire.daily.as_ref().map(|ladder| parse_ladder(ladder, Span::Day)).transpose()?,
            weekly: wire.weekly.as_ref().map(|ladder| parse_ladder(ladder, Span::Week)).transpose()?,
            rest_days: wire.rest_days.as_deref().map(|text| parse_multiplier(text, "overtime.rest_days")).transpose()?,
        })
    }

    /// The rules as the API writes them: figures without trailing zeros, the
    /// way they were meant - `"1.5"`, `"8"`, `"7.75"`.
    pub fn to_wire(&self) -> OvertimeWire {
        let ladder = |ladder: &Ladder| LadderWire {
            after: match ladder.after {
                Threshold::Norm => NORM.to_string(),
                Threshold::Hours(hours) => figure(hours),
            },
            multiplier: figure(ladder.multiplier),
            then: ladder.then.map(|step| StepWire {
                after: figure(step.after),
                multiplier: figure(step.multiplier),
            }),
        };
        OvertimeWire {
            daily: self.daily.as_ref().map(ladder),
            weekly: self.weekly.as_ref().map(ladder),
            rest_days: self.rest_days.map(figure),
        }
    }

    /// The rules a column each, as `pay_terms` stores them.
    pub fn to_columns(&self) -> Columns {
        let split = |ladder: Option<&Ladder>| match ladder {
            None => (None, None, None, None),
            Some(ladder) => (
                match ladder.after {
                    Threshold::Norm => None,
                    Threshold::Hours(hours) => Some(hours),
                },
                Some(ladder.multiplier),
                ladder.then.map(|step| step.after),
                ladder.then.map(|step| step.multiplier),
            ),
        };
        let (daily_after_hours, daily_multiplier, daily_then_after_hours, daily_then_multiplier) = split(self.daily.as_ref());
        let (weekly_after_hours, weekly_multiplier, weekly_then_after_hours, weekly_then_multiplier) = split(self.weekly.as_ref());
        Columns {
            daily_after_hours,
            daily_multiplier,
            daily_then_after_hours,
            daily_then_multiplier,
            weekly_after_hours,
            weekly_multiplier,
            weekly_then_after_hours,
            weekly_then_multiplier,
            rest_day_multiplier: self.rest_days,
        }
    }

    /// Reassembles the rules from their columns. A multiplier missing means
    /// no rule, whatever else is null - the database refuses the rest.
    pub fn from_columns(columns: &Columns) -> Self {
        let ladder = |after: Option<Decimal>, multiplier: Option<Decimal>, then_after: Option<Decimal>, then_multiplier: Option<Decimal>| {
            multiplier.map(|multiplier| Ladder {
                after: after.map_or(Threshold::Norm, Threshold::Hours),
                multiplier: multiplier.normalize(),
                then: then_after.zip(then_multiplier).map(|(after, multiplier)| Step {
                    after: after.normalize(),
                    multiplier: multiplier.normalize(),
                }),
            })
        };
        Self {
            daily: ladder(
                columns.daily_after_hours.map(|hours| hours.normalize()),
                columns.daily_multiplier,
                columns.daily_then_after_hours,
                columns.daily_then_multiplier,
            ),
            weekly: ladder(
                columns.weekly_after_hours.map(|hours| hours.normalize()),
                columns.weekly_multiplier,
                columns.weekly_then_after_hours,
                columns.weekly_then_multiplier,
            ),
            rest_days: columns.rest_day_multiplier.map(|multiplier| multiplier.normalize()),
        }
    }
}

/// The rules as stored, a column each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Columns {
    pub daily_after_hours: Option<Decimal>,
    pub daily_multiplier: Option<Decimal>,
    pub daily_then_after_hours: Option<Decimal>,
    pub daily_then_multiplier: Option<Decimal>,
    pub weekly_after_hours: Option<Decimal>,
    pub weekly_multiplier: Option<Decimal>,
    pub weekly_then_after_hours: Option<Decimal>,
    pub weekly_then_multiplier: Option<Decimal>,
    pub rest_day_multiplier: Option<Decimal>,
}

impl<'r> sqlx::FromRow<'r, PgRow> for Overtime {
    fn from_row(row: &'r PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self::from_columns(&Columns {
            daily_after_hours: row.try_get("daily_after_hours")?,
            daily_multiplier: row.try_get("daily_multiplier")?,
            daily_then_after_hours: row.try_get("daily_then_after_hours")?,
            daily_then_multiplier: row.try_get("daily_then_multiplier")?,
            weekly_after_hours: row.try_get("weekly_after_hours")?,
            weekly_multiplier: row.try_get("weekly_multiplier")?,
            weekly_then_after_hours: row.try_get("weekly_then_after_hours")?,
            weekly_then_multiplier: row.try_get("weekly_then_multiplier")?,
            rest_day_multiplier: row.try_get("rest_day_multiplier")?,
        }))
    }
}

/// A figure without trailing zeros: `1.5000` as `1.5`, `8.00` as `8`.
fn figure(value: Decimal) -> String {
    value.normalize().to_string()
}

fn parse_ladder(wire: &LadderWire, span: Span) -> Result<Ladder, ApiError> {
    let field = format!("overtime.{}", span.name());
    let after = if wire.after.trim().eq_ignore_ascii_case(NORM) {
        Threshold::Norm
    } else {
        Threshold::Hours(parse_hours(&wire.after, &format!("{field}.after"), span)?)
    };
    let multiplier = parse_multiplier(&wire.multiplier, &format!("{field}.multiplier"))?;
    let then = match &wire.then {
        None => None,
        Some(step) => {
            let step_after = parse_hours(&step.after, &format!("{field}.then.after"), span)?;
            let step_multiplier = parse_multiplier(&step.multiplier, &format!("{field}.then.multiplier"))?;
            if step_multiplier <= multiplier {
                return Err(ApiError::bad_request(format!(
                    "{field}.then.multiplier must be more than {field}.multiplier: a later step that paid less would pay the later hour less"
                )));
            }
            if let Threshold::Hours(first) = after
                && first + step_after > span.max_hours()
            {
                return Err(ApiError::bad_request(format!(
                    "{field}: the second step starts at {} hours, past the end of the {}",
                    figure(first + step_after),
                    match span {
                        Span::Day => "day",
                        Span::Week => "week",
                    }
                )));
            }
            Some(Step {
                after: step_after,
                multiplier: step_multiplier,
            })
        }
    };
    Ok(Ladder { after, multiplier, then })
}

/// Hours in a span: more than zero, at most the span, at most hundredths.
fn parse_hours(text: &str, field: &str, span: Span) -> Result<Decimal, ApiError> {
    let hours = Decimal::from_str_exact(text.trim())
        .map_err(|_| ApiError::bad_request(format!("{field} must be hours written as a string, like \"8\" or \"7.5\" - not {text:?}")))?
        .normalize();
    if hours <= Decimal::ZERO || hours > span.max_hours() {
        return Err(ApiError::bad_request(format!(
            "{field} must be more than 0 and at most {} hours",
            span.max_hours()
        )));
    }
    if hours.scale() > HOURS_SCALE {
        return Err(ApiError::bad_request(format!("{field} has more than {HOURS_SCALE} decimal places")));
    }
    Ok(hours)
}

/// A multiplier: more than one - an "overtime" that paid the same would be
/// no rule at all - at most ten, at most four decimal places.
fn parse_multiplier(text: &str, field: &str) -> Result<Decimal, ApiError> {
    let multiplier = Decimal::from_str_exact(text.trim())
        .map_err(|_| ApiError::bad_request(format!("{field} must be a decimal written as a string, like \"1.5\" - not {text:?}")))?
        .normalize();
    if multiplier <= Decimal::ONE || multiplier > Decimal::from(MULTIPLIER_MAX) {
        return Err(ApiError::bad_request(format!("{field} must be more than 1 and at most {MULTIPLIER_MAX}")));
    }
    if multiplier.scale() > MULTIPLIER_SCALE {
        return Err(ApiError::bad_request(format!("{field} has more than {MULTIPLIER_SCALE} decimal places")));
    }
    Ok(multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal(text: &str) -> Decimal {
        text.parse().expect("a decimal")
    }

    fn ladder(after: &str, multiplier: &str) -> LadderWire {
        LadderWire {
            after: after.to_string(),
            multiplier: multiplier.to_string(),
            then: None,
        }
    }

    fn with_then(mut wire: LadderWire, after: &str, multiplier: &str) -> LadderWire {
        wire.then = Some(StepWire {
            after: after.to_string(),
            multiplier: multiplier.to_string(),
        });
        wire
    }

    fn daily(wire: LadderWire) -> Result<Overtime, ApiError> {
        Overtime::parse(&OvertimeWire {
            daily: Some(wire),
            ..OvertimeWire::default()
        })
    }

    #[test]
    fn the_two_shapes_the_laws_take_are_one_shape() {
        // Russia: the first two hours past the norm at 1.5, then 2.
        let russia = daily(with_then(ladder("norm", "1.5"), "2", "2")).unwrap();
        let rule = russia.daily.unwrap();
        assert_eq!(rule.after, Threshold::Norm);
        assert_eq!(
            rule.steps(7 * 3600),
            [(7 * 3600, decimal("1.5")), (9 * 3600, decimal("2"))],
            "a short eve moves both steps"
        );

        // California: 1.5 after eight hours, 2 after twelve, whatever the norm.
        let california = daily(with_then(ladder("8", "1.5"), "4", "2")).unwrap();
        let rule = california.daily.unwrap();
        assert_eq!(rule.steps(7 * 3600), [(8 * 3600, decimal("1.5")), (12 * 3600, decimal("2"))]);
        assert_eq!(rule.steps(4 * 3600), rule.steps(8 * 3600), "a fixed threshold ignores the norm");
    }

    #[test]
    fn a_multiplier_must_pay_more_than_an_ordinary_hour() {
        assert!(daily(ladder("8", "1")).is_err(), "1 is no rule at all");
        assert!(daily(ladder("8", "0.5")).is_err());
        assert!(daily(ladder("8", "10.5")).is_err(), "past the bound on typing");
        assert!(daily(ladder("8", "1.33333")).is_err(), "a fifth decimal place would be rounded away");
        assert!(daily(ladder("8", "1.3333")).is_ok());
        assert!(daily(ladder("8", "one and a half")).is_err());
    }

    #[test]
    fn a_threshold_stays_inside_its_span() {
        assert!(daily(ladder("0", "1.5")).is_err(), "every hour is not overtime");
        assert!(daily(ladder("25", "1.5")).is_err(), "a day has 24 hours");
        assert!(daily(ladder("7.755", "1.5")).is_err(), "hundredths at most");
        assert!(daily(ladder("7.75", "1.5")).is_ok());
        assert!(daily(ladder(" NORM ", "1.5")).is_ok(), "the word is read as a person types it");

        let weekly = Overtime::parse(&OvertimeWire {
            weekly: Some(ladder("40", "1.5")),
            ..OvertimeWire::default()
        });
        assert!(weekly.is_ok(), "forty hours fit a week, not a day");
        assert!(daily(ladder("40", "1.5")).is_err());
    }

    #[test]
    fn a_second_step_pays_more_and_can_be_reached() {
        assert!(daily(with_then(ladder("8", "2"), "2", "1.5")).is_err(), "a ladder that goes down");
        assert!(daily(with_then(ladder("8", "1.5"), "2", "1.5")).is_err(), "or stays level");
        assert!(daily(with_then(ladder("20", "1.5"), "5", "2")).is_err(), "the 25th hour does not exist");
        assert!(daily(with_then(ladder("20", "1.5"), "4", "2")).is_ok(), "the 24th does");
        assert!(
            daily(with_then(ladder("norm", "1.5"), "0", "2")).is_err(),
            "a step at the same place is the first one"
        );
    }

    #[test]
    fn the_wire_reads_back_what_it_wrote() {
        let wire = OvertimeWire {
            daily: Some(with_then(ladder("norm", "1.50"), "2.00", "2")),
            weekly: Some(ladder("40", "1.5")),
            rest_days: Some("2.0".to_string()),
        };
        let rules = Overtime::parse(&wire).unwrap();
        let written = rules.to_wire();
        assert_eq!(written.daily.as_ref().unwrap().multiplier, "1.5", "no trailing zeros");
        assert_eq!(written.daily.as_ref().unwrap().then.as_ref().unwrap().after, "2");
        assert_eq!(written.rest_days.as_deref(), Some("2"));
        assert_eq!(Overtime::parse(&written).unwrap(), rules);
        assert_eq!(Overtime::from_columns(&rules.to_columns()), rules, "and the columns hold all of it");
    }

    #[test]
    fn no_rules_is_no_overtime() {
        let none = Overtime::parse(&OvertimeWire::default()).unwrap();
        assert!(none.is_none());
        assert_eq!(none.to_columns(), Columns::default(), "every column null, as the versions before 0.32 are");
        assert_eq!(Overtime::from_columns(&Columns::default()), Overtime::default());
    }

    #[test]
    fn unknown_fields_are_refused() {
        // A typo in a rule would otherwise vanish, and with it the overtime a
        // person believes was recorded.
        let typo = serde_json::from_value::<OvertimeWire>(serde_json::json!({ "dayly": { "after": "8", "multiplier": "1.5" } }));
        assert!(typo.is_err());
        let typo = serde_json::from_value::<OvertimeWire>(serde_json::json!({ "daily": { "after": "8", "multiplier": "1.5", "ten": "2" } }));
        assert!(typo.is_err());
    }
}
