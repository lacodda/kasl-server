//! The privacy manifest: what this installation stores about a person, and the
//! setting that decides it.
//!
//! An employee is asked to run an agent that notices when they stop typing.
//! The only honest answer to "what does it send" is one the server enforces
//! and can recite, so this module is two halves of the same promise: a level
//! applied at ingest, and a manifest generated from that same level rather
//! than written by hand (ADR 0011).
//!
//! Filtering happens on the way in. A field a level excludes is dropped before
//! the day is written, so it never reaches the database or a backup - the
//! promise is about the disk, not about what a screen chooses to show.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool};

use crate::{app::AppState, audit, auth::AuthenticatedAgent, error::ApiError, login::CurrentUser};

/// How much detail the installation keeps. Mirrors the `privacy_level` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "privacy_level", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum PrivacyLevel {
    /// Everything the agent sends. What every version before 0.10.0 did, and
    /// the default: narrowing is the deliberate act.
    Full,
    /// Times without the words. Pauses keep when they happened but not the
    /// reason typed into them; tasks keep their names but not their comments.
    Moderate,
    /// Hours, not a timeline. A day keeps its start, its end, and how much of
    /// it was paused as a count and a total; individual pauses and tasks are
    /// not stored at all.
    Coarse,
}

impl PrivacyLevel {
    /// Whether free text the employee typed is kept - pause reasons and task
    /// comments. These go first because they are the only fields where a
    /// person writes about themselves in their own words.
    pub fn keeps_free_text(self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether individual pauses are stored, rather than summarized.
    pub fn keeps_pause_times(self) -> bool {
        matches!(self, Self::Full | Self::Moderate)
    }

    /// Whether tasks are stored at all.
    pub fn keeps_tasks(self) -> bool {
        matches!(self, Self::Full | Self::Moderate)
    }
}

/// The installation's settings as stored. One row.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct Policy {
    pub privacy_level: PrivacyLevel,
}

impl Policy {
    /// Reads the policy. Called once per upload - including once for a whole
    /// batch, not once per day in it.
    pub async fn load(executor: impl PgExecutor<'_>) -> Result<Self, ApiError> {
        let policy: Policy = sqlx::query_as("SELECT privacy_level FROM settings WHERE singleton").fetch_one(executor).await?;
        Ok(policy)
    }

    pub fn level(self) -> PrivacyLevel {
        self.privacy_level
    }
}

/// What a level did to one day on its way in.
///
/// Reported back to the agent so a delivery it believes in matches what was
/// stored: told "5 pauses accepted" under a policy that kept none, an agent
/// would report a break as recorded when it was not (ADR 0011).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Dropped {
    /// Pauses summarized away instead of stored individually.
    #[serde(skip_serializing_if = "is_zero")]
    pub pauses: usize,
    /// Tasks not stored at all.
    #[serde(skip_serializing_if = "is_zero")]
    pub tasks: usize,
    /// Free-text fields cleared: pause reasons plus task comments.
    #[serde(skip_serializing_if = "is_zero")]
    pub free_text: usize,
}

impl Dropped {
    /// Takes a reference so serde's `skip_serializing_if` can call it directly.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

/// The manifest: what is stored, who sees it, what is never collected.
///
/// Generated from the level so it cannot drift from what the server does.
#[derive(Debug, Serialize)]
pub struct Manifest {
    pub level: PrivacyLevel,
    /// One line an employee can read without knowing the levels exist.
    pub summary: &'static str,
    /// What is kept, field by field, in the words of the thing itself.
    pub stored: Vec<Stored>,
    /// Named explicitly, because a reader cannot tell "we do not collect this"
    /// from "this was left off the list".
    pub never_collected: Vec<&'static str>,
    /// Who can see a given person's data.
    pub visible_to: Vec<&'static str>,
    /// How long it is kept, stated plainly rather than implied.
    pub retention: &'static str,
    /// What changing the level does - and does not do - to what is already
    /// stored. The hopeful reading is the wrong one.
    pub on_change: &'static str,
    pub updated_at: Option<DateTime<Utc>>,
}

/// One kind of data the server holds.
#[derive(Debug, Serialize)]
pub struct Stored {
    pub what: &'static str,
    pub detail: &'static str,
}

/// Everything the agent could send, and what each level does with it.
///
/// One list rather than a branch per level: a new field is described once, and
/// describing it under some levels but not others is not possible.
fn stored_at(level: PrivacyLevel) -> Vec<Stored> {
    let mut stored = vec![
        Stored {
            what: "workdays",
            detail: "the date, when the day started, and when it ended",
        },
        Stored {
            what: "pauses",
            detail: if level.keeps_pause_times() {
                "each interruption: when it began, how long it lasted, and whether it was a break you entered yourself"
            } else {
                "how many times the day was interrupted and for how long in total - not when"
            },
        },
    ];

    if level.keeps_tasks() {
        stored.push(Stored {
            what: "tasks",
            detail: if level.keeps_free_text() {
                "what you logged: the name, your comment, and how complete you marked it"
            } else {
                "what you logged: the name and how complete you marked it - not your comment"
            },
        });
    }

    if level.keeps_free_text() {
        stored.push(Stored {
            what: "pause reasons",
            detail: "the text you type when you take a break by hand",
        });
    }

    stored.push(Stored {
        what: "account",
        detail: "your email, display name, role, department, and which machines report for you",
    });

    stored
}

/// Things the server has no column for. Absence is not reassuring on its own.
const NEVER_COLLECTED: [&str; 7] = [
    "keystrokes or what you type",
    "window titles",
    "which applications you run",
    "screenshots or camera images",
    "web pages you visit",
    "file names or paths",
    "your location",
];

fn summary_for(level: PrivacyLevel) -> &'static str {
    match level {
        PrivacyLevel::Full => {
            "This server stores your working hours, every interruption with the reason you gave for it, and the tasks you logged with their comments."
        }
        PrivacyLevel::Moderate => {
            "This server stores your working hours, when you were interrupted, and the names of tasks you logged - but none of the text you typed about them."
        }
        PrivacyLevel::Coarse => "This server stores your working hours and how much of the day you were away - not when, and not what you worked on.",
    }
}

/// Builds the manifest for a level.
pub fn manifest(level: PrivacyLevel, updated_at: Option<DateTime<Utc>>) -> Manifest {
    Manifest {
        level,
        summary: summary_for(level),
        stored: stored_at(level),
        never_collected: NEVER_COLLECTED.to_vec(),
        visible_to: vec![
            "you, in your own account",
            "the manager of your department",
            "administrators of this installation",
        ],
        retention: "Kept for as long as the installation keeps it: there is no automatic deletion. A deactivated account keeps its history rather than losing it.",
        on_change: "Changing this setting affects what arrives from now on. Narrowing it does not erase what is already stored, and widening it does not bring back what was dropped.",
        updated_at,
    }
}

/// The level being set.
#[derive(Debug, Deserialize)]
pub struct LevelUpdate {
    pub level: PrivacyLevel,
}

/// Answers the manifest to a signed-in person.
pub async fn show(State(state): State<AppState>, _user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(current(&state.pool).await?))
}

/// Answers the manifest to an authenticated agent.
///
/// The point of the agent route: kasl can show the manifest in the CLI, where
/// the employee already is, instead of asking them to sign into the server
/// that watches them in order to find out what it watches.
pub async fn show_to_agent(State(state): State<AppState>, _agent: AuthenticatedAgent) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(current(&state.pool).await?))
}

async fn current(pool: &PgPool) -> Result<Manifest, ApiError> {
    let row: (PrivacyLevel, DateTime<Utc>) = sqlx::query_as("SELECT privacy_level, updated_at FROM settings WHERE singleton")
        .fetch_one(pool)
        .await?;
    Ok(manifest(row.0, Some(row.1)))
}

/// Sets the level. Administrators only, and recorded.
pub async fn update(State(state): State<AppState>, user: CurrentUser, Json(update): Json<LevelUpdate>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let previous: PrivacyLevel = sqlx::query_scalar("SELECT privacy_level FROM settings WHERE singleton")
        .fetch_one(&state.pool)
        .await?;

    sqlx::query("UPDATE settings SET privacy_level = $1 WHERE singleton")
        .bind(update.level)
        .execute(&state.pool)
        .await?;

    tracing::info!(from = ?previous, to = ?update.level, by = %user.user_id, "changed the privacy level");
    // A policy that can be quietly loosened is not a policy (ADR 0011).
    audit::Entry::new(audit::action::PRIVACY_LEVEL_CHANGED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({ "from": previous, "to": update.level }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::OK, Json(current(&state.pool).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_level_keeps_everything() {
        // The regression this guards: a well-meaning change to the default
        // would start discarding data in installations already running, and
        // what is dropped at ingest cannot be recovered (ADR 0011).
        assert!(PrivacyLevel::Full.keeps_free_text());
        assert!(PrivacyLevel::Full.keeps_pause_times());
        assert!(PrivacyLevel::Full.keeps_tasks());
    }

    #[test]
    fn levels_narrow_in_one_direction() {
        // Each level keeps a subset of the one above it. A level that kept
        // something a wider level dropped would make "narrowing" meaningless.
        let levels = [PrivacyLevel::Full, PrivacyLevel::Moderate, PrivacyLevel::Coarse];
        let keeps: [fn(PrivacyLevel) -> bool; 3] = [PrivacyLevel::keeps_free_text, PrivacyLevel::keeps_pause_times, PrivacyLevel::keeps_tasks];
        for pair in levels.windows(2) {
            let (wider, narrower) = (pair[0], pair[1]);
            for keeps in keeps {
                assert!(keeps(wider) || !keeps(narrower), "{narrower:?} keeps something {wider:?} does not");
            }
        }
    }

    #[test]
    fn the_wire_names_are_the_contract() {
        // kasl parses these, and the manifest is shown to people through it.
        assert_eq!(serde_json::to_string(&PrivacyLevel::Full).unwrap(), "\"full\"");
        assert_eq!(serde_json::to_string(&PrivacyLevel::Moderate).unwrap(), "\"moderate\"");
        assert_eq!(serde_json::to_string(&PrivacyLevel::Coarse).unwrap(), "\"coarse\"");
    }

    #[test]
    fn a_narrower_manifest_promises_less() {
        // The manifest is generated from the level, so this is really a test
        // that generation is wired to the level at all - a hand-written
        // manifest that ignored its argument would pass every other test here.
        let full = manifest(PrivacyLevel::Full, None);
        let coarse = manifest(PrivacyLevel::Coarse, None);

        assert!(full.stored.iter().any(|s| s.what == "tasks"), "full stores tasks");
        assert!(!coarse.stored.iter().any(|s| s.what == "tasks"), "coarse stores no tasks");
        assert!(full.stored.iter().any(|s| s.what == "pause reasons"));
        assert!(!coarse.stored.iter().any(|s| s.what == "pause reasons"));
        assert_ne!(full.summary, coarse.summary);
    }

    #[test]
    fn every_level_names_what_is_never_collected() {
        // The list does not depend on the level: no level of this product
        // watches keystrokes, and a reader at `full` needs to know that most.
        for level in [PrivacyLevel::Full, PrivacyLevel::Moderate, PrivacyLevel::Coarse] {
            let manifest = manifest(level, None);
            assert_eq!(manifest.never_collected.len(), NEVER_COLLECTED.len());
            assert!(manifest.never_collected.contains(&"keystrokes or what you type"));
        }
    }

    #[test]
    fn dropped_counts_stay_out_of_an_untouched_response() {
        // The common upload is at `full`, where nothing is dropped. Serializing
        // three zeroes onto every accepted day would train a reader to ignore
        // the field that exists to be noticed.
        let json = serde_json::to_value(Dropped::default()).unwrap();
        assert_eq!(json, serde_json::json!({}));
        assert!(Dropped::default().is_empty());

        let json = serde_json::to_value(Dropped {
            pauses: 2,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({ "pauses": 2 }));
    }
}
