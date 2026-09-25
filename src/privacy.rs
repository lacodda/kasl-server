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
use sqlx::PgExecutor;

use crate::{
    app::AppState,
    audit,
    auth::AuthenticatedAgent,
    error::ApiError,
    login::CurrentUser,
    webhooks::{EventKind, Kind, Webhooks},
};

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
    /// What leaves this server on its own, and where to. Always present, and
    /// empty when nothing does - an absent list and an empty one would read
    /// the same, and only one of them is a promise (ADR 0019).
    pub sent_elsewhere: Vec<SentElsewhere>,
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

/// One place the server sends things about people, told in the employee's
/// terms rather than the operator's.
#[derive(Debug, Serialize)]
pub struct SentElsewhere {
    /// Where, by kind and by the name the operator gave it: "a Slack channel
    /// (team)". Never the address.
    pub to: String,
    /// About whom: everyone, or the people of one department.
    pub about: String,
    /// What each message carries.
    pub what: Vec<&'static str>,
}

/// The destinations, as the manifest tells them.
///
/// Built from the same configuration the dispatcher sends through, so the
/// manifest cannot list a channel that is not there or miss one that is.
fn sent_elsewhere(webhooks: &Webhooks) -> Vec<SentElsewhere> {
    webhooks
        .destinations()
        .iter()
        .map(|destination| {
            let place = match destination.kind {
                Kind::Slack => "a Slack channel",
                Kind::Mattermost => "a Mattermost channel",
                Kind::Telegram => "a Telegram chat",
                Kind::Json => "another system the operator runs",
            };
            let mut what = Vec::new();
            if destination.events.iter().any(|event| matches!(event, EventKind::AlertRaised | EventKind::AlertAcknowledged | EventKind::AlertResolved)) {
                what.push(
                    "alerts about you as they are raised and cleared: your name, your department, and the figure behind each one - how long your agent was quiet, how long a day ran against your norm, or how long a day stayed open",
                );
            }
            if destination.hears(EventKind::AlertAcknowledged) {
                what.push("the name of whoever answered an alert about you");
            }
            if destination.hears(EventKind::DayClosed) {
                what.push("each day you finish: your name, your department, the date, when it started and ended, and the hours worked - or that you marked it as leave, sick or a day off");
            }
            SentElsewhere {
                to: format!("{place} ({})", destination.name),
                about: match &destination.department {
                    Some(department) => format!("people in {department}"),
                    None => "everyone".to_string(),
                },
                what,
            }
        })
        .collect()
}

/// Everything the agent could send, and what each level does with it.
///
/// One list rather than a branch per level: a new field is described once, and
/// describing it under some levels but not others is not possible.
fn stored_at(level: PrivacyLevel) -> Vec<Stored> {
    let mut stored = vec![
        Stored {
            what: "workdays",
            // The kind is named at every level. It is the one field here that
            // says something about a person's life rather than their keyboard
            // - "off sick" is a fact about them - so a manifest that listed
            // the times and left it out would be describing a quieter server
            // than the one running (ADR 0011).
            detail: "the date, when the day started, when it ended, and whether you marked it as leave, sick or a day off",
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

    // Listed at every level, and worded as what it is. The pulse is the one
    // thing here that is about the present moment rather than a day already
    // over, so a manifest that mentioned only days would be describing a
    // quieter server than the one running (ADR 0014).
    stored.push(Stored {
        what: "live status",
        detail: "whether your agent currently reports you as working, on a break, or not in a day - the latest one only, replaced each time it arrives, never kept as a history",
    });

    // What the server told the person is itself something kept about them,
    // and a manifest that left it out would describe a quieter server than
    // the one running (ADR 0020). Worded with who reads it, because that is
    // the question it raises.
    stored.push(Stored {
        what: "notifications",
        detail: "what this server has told you - an alert about you, a machine added to or removed from your account, a change to this page - and how far you have read; readable by you alone",
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

/// The level in one sentence. Shared with the notice that tells people it
/// changed, so the toast and the manifest describe a level in the same words.
pub fn summary_for(level: PrivacyLevel) -> &'static str {
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
pub fn manifest(level: PrivacyLevel, updated_at: Option<DateTime<Utc>>, webhooks: &Webhooks) -> Manifest {
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
        sent_elsewhere: sent_elsewhere(webhooks),
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
    Ok(Json(current(&state).await?))
}

/// Answers the manifest to an authenticated agent.
///
/// The point of the agent route: kasl can show the manifest in the CLI, where
/// the employee already is, instead of asking them to sign into the server
/// that watches them in order to find out what it watches.
pub async fn show_to_agent(State(state): State<AppState>, _agent: AuthenticatedAgent) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(current(&state).await?))
}

async fn current(state: &AppState) -> Result<Manifest, ApiError> {
    let row: (PrivacyLevel, DateTime<Utc>) = sqlx::query_as("SELECT privacy_level, updated_at FROM settings WHERE singleton")
        .fetch_one(&state.pool)
        .await?;
    Ok(manifest(row.0, Some(row.1), &state.webhooks))
}

/// Sets the level. Administrators only, and recorded.
pub async fn update(State(state): State<AppState>, user: CurrentUser, Json(update): Json<LevelUpdate>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let previous: PrivacyLevel = sqlx::query_scalar("SELECT privacy_level FROM settings WHERE singleton")
        .fetch_one(&state.pool)
        .await?;

    // Everybody is told, in the same transaction: a manifest that changes
    // without a word is one nobody can rely on (ADR 0020). Setting the level it
    // already has changes nothing, and says nothing.
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE settings SET privacy_level = $1 WHERE singleton")
        .bind(update.level)
        .execute(&mut *tx)
        .await?;
    if previous != update.level {
        crate::notifications::privacy_changed(&mut tx, previous, update.level).await?;
    }
    tx.commit().await?;

    tracing::info!(from = ?previous, to = ?update.level, by = %user.user_id, "changed the privacy level");
    // A policy that can be quietly loosened is not a policy (ADR 0011).
    audit::Entry::new(audit::action::PRIVACY_LEVEL_CHANGED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({ "from": previous, "to": update.level }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::OK, Json(current(&state).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_leaves_the_server_is_listed_from_the_configuration() {
        // The failure this guards is a manifest that says less than the
        // server does: a channel that hears about somebody's days, missing
        // from the one page that promises to say what happens to them.
        let none = manifest(PrivacyLevel::Full, None, &Webhooks::default());
        assert!(none.sent_elsewhere.is_empty());

        let webhooks = Webhooks::new(
            vec![
                crate::webhooks::Destination::parse("KASL_WEBHOOK_TEAM", "slack https://hooks.slack.com/services/T/B/X").unwrap(),
                crate::webhooks::Destination::parse("KASL_WEBHOOK_PAY", "json https://pay.example/in secret=s events=day.closed department=Design").unwrap(),
            ],
            None,
        );
        let sent = manifest(PrivacyLevel::Full, None, &webhooks).sent_elsewhere;
        assert_eq!(sent.len(), 2);

        let pay = sent.iter().find(|s| s.to.contains("(pay)")).expect("the json destination is listed");
        assert_eq!(pay.about, "people in Design");
        assert_eq!(pay.what.len(), 1, "a destination hearing only days is not said to hear alerts");
        assert!(pay.what[0].contains("hours worked"));

        let team = sent.iter().find(|s| s.to.contains("(team)")).expect("the slack destination is listed");
        assert_eq!(team.to, "a Slack channel (team)");
        assert_eq!(team.about, "everyone");
        assert!(team.what.iter().any(|w| w.contains("alerts about you")));
        assert!(!team.what.iter().any(|w| w.contains("each day you finish")), "days are opt-in");
        assert!(!format!("{sent:?}").contains("hooks.slack.com"), "the manifest never carries an address");
    }

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
        let full = manifest(PrivacyLevel::Full, None, &Webhooks::default());
        let coarse = manifest(PrivacyLevel::Coarse, None, &Webhooks::default());

        assert!(full.stored.iter().any(|s| s.what == "tasks"), "full stores tasks");
        assert!(!coarse.stored.iter().any(|s| s.what == "tasks"), "coarse stores no tasks");
        assert!(full.stored.iter().any(|s| s.what == "pause reasons"));
        assert!(!coarse.stored.iter().any(|s| s.what == "pause reasons"));
        assert_ne!(full.summary, coarse.summary);
    }

    #[test]
    fn every_level_names_the_live_status() {
        // The pulse is not governed by the level - narrowing to `coarse` stops
        // the server storing when you paused, not the agent telling it you are
        // paused right now. A manifest that left it out would be describing a
        // server that watches less than this one does (ADR 0014).
        for level in [PrivacyLevel::Full, PrivacyLevel::Moderate, PrivacyLevel::Coarse] {
            assert!(
                manifest(level, None, &Webhooks::default()).stored.iter().any(|s| s.what == "live status"),
                "{level:?} does not name the pulse",
            );
        }
    }

    #[test]
    fn every_level_names_the_notifications() {
        // What the server told a person is kept about them at every level, and
        // who reads it is the question a reader brings (ADR 0020).
        for level in [PrivacyLevel::Full, PrivacyLevel::Moderate, PrivacyLevel::Coarse] {
            let manifest = manifest(level, None, &Webhooks::default());
            let notices = manifest
                .stored
                .iter()
                .find(|s| s.what == "notifications")
                .unwrap_or_else(|| panic!("{level:?} does not name the notifications"));
            assert!(notices.detail.contains("you alone"), "{}", notices.detail);
        }
    }

    #[test]
    fn every_level_names_what_is_never_collected() {
        // The list does not depend on the level: no level of this product
        // watches keystrokes, and a reader at `full` needs to know that most.
        for level in [PrivacyLevel::Full, PrivacyLevel::Moderate, PrivacyLevel::Coarse] {
            let manifest = manifest(level, None, &Webhooks::default());
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
