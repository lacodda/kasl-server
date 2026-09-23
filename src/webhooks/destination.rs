//! Where events go: destinations read from the deployment's environment.
//!
//! One variable per destination, `KASL_WEBHOOK_<NAME>`, and the name is the
//! label the rest of the server uses - in the delivery log, on the screen, in
//! the privacy manifest. The address itself never leaves this module except
//! into the request that uses it (ADR 0019).
//!
//! The value is a kind, a target, and options:
//!
//! ```text
//! KASL_WEBHOOK_TEAM=slack https://hooks.slack.com/services/T0/B0/XXXX
//! KASL_WEBHOOK_DESIGN=mattermost https://chat.example.com/hooks/xxx department=Design
//! KASL_WEBHOOK_OPS=telegram 123456:ABC-DEF chat=-1001234567890 events=alert.raised,alert.resolved
//! KASL_WEBHOOK_PAYROLL=json https://payroll.example.com/kasl secret=... events=day.closed
//! ```
//!
//! A value that does not parse stops the server from starting. A destination
//! silently dropped because of a typo is a channel that never hears about the
//! agent that died, and nothing anywhere would say so.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The prefix every destination's variable starts with.
pub const PREFIX: &str = "KASL_WEBHOOK_";

/// Telegram's Bot API, where a `telegram` destination posts.
const TELEGRAM_API: &str = "https://api.telegram.org";

/// What a destination speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A Slack incoming webhook.
    Slack,
    /// A Mattermost incoming webhook. Accepts Slack's shape, but reads
    /// Markdown rather than Slack's own markup, so the text is rendered apart.
    Mattermost,
    /// A Telegram chat, through a bot the operator created.
    Telegram,
    /// The event itself as JSON, signed - for a system of the operator's own.
    Json,
}

impl Kind {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "slack" => Ok(Self::Slack),
            "mattermost" => Ok(Self::Mattermost),
            "telegram" => Ok(Self::Telegram),
            "json" => Ok(Self::Json),
            other => Err(format!("unknown kind `{}`: expected slack, mattermost, telegram or json", first_word(other))),
        }
    }
}

/// What happened. Mirrors the `webhook_event` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "webhook_event")]
pub enum EventKind {
    #[serde(rename = "alert.raised")]
    #[sqlx(rename = "alert.raised")]
    AlertRaised,
    #[serde(rename = "alert.acknowledged")]
    #[sqlx(rename = "alert.acknowledged")]
    AlertAcknowledged,
    #[serde(rename = "alert.resolved")]
    #[sqlx(rename = "alert.resolved")]
    AlertResolved,
    #[serde(rename = "day.closed")]
    #[sqlx(rename = "day.closed")]
    DayClosed,
    #[serde(rename = "test")]
    #[sqlx(rename = "test")]
    Test,
}

impl EventKind {
    /// Every event a destination can subscribe to. `test` is not among them:
    /// it is sent only when an administrator asks, to the one destination they
    /// asked about, whatever it subscribes to.
    pub const SUBSCRIBABLE: [Self; 4] = [Self::AlertRaised, Self::AlertAcknowledged, Self::AlertResolved, Self::DayClosed];

    /// What a destination hears when it names no events: the life of an
    /// alert, and not the days. An alert is rare and a day closes for every
    /// person every day - a channel that opts into that should have done so
    /// on purpose.
    const DEFAULT: [Self; 3] = [Self::AlertRaised, Self::AlertAcknowledged, Self::AlertResolved];

    pub fn name(self) -> &'static str {
        match self {
            Self::AlertRaised => "alert.raised",
            Self::AlertAcknowledged => "alert.acknowledged",
            Self::AlertResolved => "alert.resolved",
            Self::DayClosed => "day.closed",
            Self::Test => "test",
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        Self::SUBSCRIBABLE.into_iter().find(|kind| kind.name() == raw).ok_or_else(|| {
            let known: Vec<&str> = Self::SUBSCRIBABLE.iter().map(|kind| kind.name()).collect();
            format!("unknown event `{}`: expected one of {}", first_word(raw), known.join(", "))
        })
    }
}

/// Where a destination's requests go. Private, and printed by nobody: every
/// variant carries a credential.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Target {
    /// Slack and Mattermost: the hook URL is the whole credential.
    Hook { url: String },
    /// Telegram: a bot token and the chat it posts into. `api` is the Bot
    /// API's address - fixed in production, pointed at a local listener by
    /// the tests.
    Telegram { token: String, chat: String, api: String },
    /// A receiver of the operator's own, and the secret the body is signed
    /// with.
    Json { url: String, secret: String },
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The one thing a `{:?}` in a log line must not do is print a hook.
        f.write_str("Target(..)")
    }
}

/// One place events are sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// The label: `KASL_WEBHOOK_TEAM_CHAT` is `team-chat`.
    pub name: String,
    pub kind: Kind,
    pub(super) target: Target,
    /// What it hears.
    pub events: Vec<EventKind>,
    /// The one department it is about, when it is about one. A channel for
    /// the design team hears about the design team: the same boundary every
    /// screen draws around a manager (ADR 0009).
    pub department: Option<String>,
}

impl Destination {
    /// Reads one destination from its variable.
    ///
    /// `variable` is the full name, `KASL_WEBHOOK_TEAM`. Errors name the
    /// variable and never repeat its value - the value is a credential, and
    /// an error message ends up in a log.
    pub fn parse(variable: &str, value: &str) -> Result<Self, String> {
        let name = label(variable)?;
        let fail = |reason: String| format!("{variable}: {reason}");

        let tokens = tokenize(value).map_err(fail)?;
        let mut tokens = tokens.into_iter();
        let kind = Kind::parse(&tokens.next().ok_or_else(|| fail("is empty; expected a kind and a target".into()))?).map_err(fail)?;
        let target = tokens.next().ok_or_else(|| fail("names a kind and no target".into()))?;

        let mut events: Option<Vec<EventKind>> = None;
        let mut department = None;
        let mut chat = None;
        let mut secret = None;
        for option in tokens {
            let Some((key, value)) = option.split_once('=') else {
                return Err(fail(format!("`{}` is not an option; options are written key=value", first_word(&option))));
            };
            let slot = match key {
                "events" => {
                    if events.is_some() {
                        return Err(fail("names `events` twice".into()));
                    }
                    let parsed = value
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(EventKind::parse)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(fail)?;
                    if parsed.is_empty() {
                        return Err(fail("`events=` lists nothing; leave it out to hear about alerts".into()));
                    }
                    events = Some(parsed);
                    continue;
                }
                "department" => &mut department,
                "chat" => &mut chat,
                "secret" => &mut secret,
                other => {
                    return Err(fail(format!(
                        "unknown option `{}`: expected events, department, chat or secret",
                        first_word(other)
                    )));
                }
            };
            if slot.is_some() {
                return Err(fail(format!("names `{key}` twice")));
            }
            if value.is_empty() {
                return Err(fail(format!("`{key}=` is empty")));
            }
            *slot = Some(value.to_string());
        }

        // Options that belong to one kind are refused on the others rather
        // than ignored: `chat=` on a Slack hook is somebody who meant
        // Telegram, and a `secret=` nobody checks is a false sense of one.
        if chat.is_some() && kind != Kind::Telegram {
            return Err(fail("`chat=` belongs to a telegram destination".into()));
        }
        if secret.is_some() && kind != Kind::Json {
            return Err(fail("`secret=` belongs to a json destination".into()));
        }

        let target = match kind {
            Kind::Slack | Kind::Mattermost => Target::Hook {
                url: http_url(&target).map_err(fail)?,
            },
            Kind::Telegram => {
                // A bot token is `<digits>:<letters>`. Checked for shape so a
                // chat id pasted into the token's place is caught here rather
                // than as a 404 from Telegram at the first alert.
                let well_formed = target
                    .split_once(':')
                    .is_some_and(|(bot, key)| !bot.is_empty() && bot.bytes().all(|b| b.is_ascii_digit()) && !key.is_empty());
                if !well_formed {
                    return Err(fail("the target of a telegram destination is the bot token, `123456:ABC...`".into()));
                }
                let chat = chat.ok_or_else(|| fail("a telegram destination needs `chat=` - the chat id the bot posts into".into()))?;
                Target::Telegram {
                    token: target,
                    chat,
                    api: TELEGRAM_API.to_string(),
                }
            }
            Kind::Json => {
                // Required, not recommended. An unsigned body is one anybody
                // who learns the address can forge, and a receiver that pays
                // or pages on these cannot tell.
                let secret = secret.ok_or_else(|| fail("a json destination needs `secret=` - the receiver checks the signature with it".into()))?;
                Target::Json {
                    url: http_url(&target).map_err(fail)?,
                    secret,
                }
            }
        };

        Ok(Self {
            name,
            kind,
            target,
            events: events.unwrap_or_else(|| EventKind::DEFAULT.to_vec()),
            department,
        })
    }

    /// Whether this destination hears about `event`.
    pub fn hears(&self, event: EventKind) -> bool {
        self.events.contains(&event)
    }

    /// Where it goes, in words safe to show: the host of a hook, the chat of
    /// a bot. Enough to tell two destinations apart and to notice one pointed
    /// at the wrong place, and never enough to post through it.
    pub fn shown_target(&self) -> String {
        match &self.target {
            Target::Hook { url } | Target::Json { url, .. } => host(url).to_string(),
            Target::Telegram { chat, .. } => format!("chat {chat}"),
        }
    }

    /// Removes the credential from a piece of text before it is kept.
    ///
    /// The dispatcher strips URLs from its errors already; this is the second
    /// lock, for the text nobody predicted - a proxy that echoes the request
    /// line, a library that changes what its errors say.
    pub fn redact(&self, text: &str) -> String {
        let secrets: Vec<&str> = match &self.target {
            Target::Hook { url } => vec![url.as_str()],
            Target::Telegram { token, .. } => vec![token.as_str()],
            Target::Json { url, secret } => vec![url.as_str(), secret.as_str()],
        };
        secrets
            .into_iter()
            .filter(|secret| !secret.is_empty())
            .fold(text.to_string(), |text, secret| text.replace(secret, "…"))
    }

    /// Points a telegram destination at another Bot API. For the tests, which
    /// cannot talk to Telegram and must not.
    #[doc(hidden)]
    pub fn with_telegram_api(mut self, api: &str) -> Self {
        if let Target::Telegram { api: current, .. } = &mut self.target {
            *current = api.trim_end_matches('/').to_string();
        }
        self
    }
}

/// The label a variable gives its destination.
fn label(variable: &str) -> Result<String, String> {
    let suffix = variable.strip_prefix(PREFIX).unwrap_or_default();
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
        return Err(format!(
            "{variable}: a destination is named `{PREFIX}<NAME>`, in capital letters, digits and underscores"
        ));
    }
    Ok(suffix.to_ascii_lowercase().replace('_', "-"))
}

/// Splits on whitespace, keeping a double-quoted run together: a department
/// is called "Customer Success" as often as it is called "Design".
fn tokenize(value: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in value.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("has a quote that is never closed".into());
    }
    if started {
        tokens.push(current);
    }
    Ok(tokens)
}

/// A word from the value, fit to quote in an error: whole when it is short
/// enough to be a typo of an event or an option, cut to its first characters
/// when it is long enough to be a credential pasted into the wrong place.
fn first_word(token: &str) -> String {
    const QUOTABLE: usize = 20;
    if token.chars().count() <= QUOTABLE {
        token.to_string()
    } else {
        format!("{}…", token.chars().take(8).collect::<String>())
    }
}

/// Accepts an `https://` or `http://` address. Plain http is allowed because
/// a Mattermost on the office network often has no certificate, and refusing
/// it would push that operator towards something worse than a LAN hop.
fn http_url(raw: &str) -> Result<String, String> {
    let rest = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://"));
    match rest {
        Some(rest) if !host(raw).is_empty() && !rest.starts_with('/') => Ok(raw.to_string()),
        _ => Err("the target is not an http(s) address".into()),
    }
}

/// The host of an address, without scheme, port, path or credentials.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map(|(_, host)| host).unwrap_or(authority);
    authority.split(':').next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOK: &str = "https://hooks.slack.com/services/T000/B000/SECRETSECRETSECRETSECRET";

    #[test]
    fn a_slack_hook_hears_about_alerts_by_default() {
        let destination = Destination::parse("KASL_WEBHOOK_TEAM_CHAT", &format!("slack {HOOK}")).unwrap();
        assert_eq!(destination.name, "team-chat");
        assert_eq!(destination.kind, Kind::Slack);
        assert!(destination.hears(EventKind::AlertRaised));
        assert!(destination.hears(EventKind::AlertResolved));
        assert!(
            !destination.hears(EventKind::DayClosed),
            "a day closes for everyone every day; hearing that is opted into"
        );
        assert_eq!(destination.department, None);
    }

    #[test]
    fn options_narrow_what_it_hears_and_about_whom() {
        let destination = Destination::parse(
            "KASL_WEBHOOK_CS",
            &format!(r#"mattermost {HOOK} events=day.closed,alert.raised department="Customer Success""#),
        )
        .unwrap();
        assert_eq!(destination.events, vec![EventKind::DayClosed, EventKind::AlertRaised]);
        assert_eq!(destination.department.as_deref(), Some("Customer Success"));
    }

    #[test]
    fn a_telegram_destination_needs_a_bot_token_and_a_chat() {
        let destination = Destination::parse("KASL_WEBHOOK_OPS", "telegram 123456:ABC-DEF chat=-100200300").unwrap();
        assert_eq!(destination.shown_target(), "chat -100200300");

        let error = Destination::parse("KASL_WEBHOOK_OPS", "telegram 123456:ABC-DEF").unwrap_err();
        assert!(error.contains("chat="), "{error}");

        // A chat id where the token belongs: caught now, not as a 404 at the
        // first alert.
        let error = Destination::parse("KASL_WEBHOOK_OPS", "telegram -100200300 chat=-100200300").unwrap_err();
        assert!(error.contains("bot token"), "{error}");
    }

    #[test]
    fn a_json_destination_is_signed_or_refused() {
        let error = Destination::parse("KASL_WEBHOOK_PAYROLL", "json https://payroll.example.com/in").unwrap_err();
        assert!(error.contains("secret="), "{error}");
        assert!(Destination::parse("KASL_WEBHOOK_PAYROLL", "json https://payroll.example.com/in secret=abc").is_ok());
    }

    #[test]
    fn an_option_for_another_kind_is_refused_rather_than_ignored() {
        let error = Destination::parse("KASL_WEBHOOK_TEAM", &format!("slack {HOOK} chat=1")).unwrap_err();
        assert!(error.contains("telegram"), "{error}");
        let error = Destination::parse("KASL_WEBHOOK_TEAM", &format!("slack {HOOK} secret=1")).unwrap_err();
        assert!(error.contains("json"), "{error}");
    }

    #[test]
    fn mistakes_are_refused_with_the_variable_named() {
        for (value, expected) in [
            ("", "empty"),
            ("discord https://x.example", "unknown kind"),
            ("slack", "no target"),
            ("slack ftp://x.example", "http(s)"),
            ("slack https://", "http(s)"),
            (&format!("slack {HOOK} events=alert.exploded"), "unknown event"),
            (&format!("slack {HOOK} events=test"), "unknown event"),
            (&format!("slack {HOOK} events="), "lists nothing"),
            (&format!("slack {HOOK} events=day.closed events=alert.raised"), "twice"),
            (&format!("slack {HOOK} colour=red"), "unknown option"),
            (&format!("slack {HOOK} department=\"Design"), "never closed"),
            (&format!("slack {HOOK} department="), "empty"),
        ] {
            let error = Destination::parse("KASL_WEBHOOK_TEAM", value).unwrap_err();
            assert!(error.starts_with("KASL_WEBHOOK_TEAM:"), "the variable is named: {error}");
            assert!(error.contains(expected), "`{value}` should say {expected}: {error}");
        }

        let error = Destination::parse("KASL_WEBHOOK_", &format!("slack {HOOK}")).unwrap_err();
        assert!(error.contains("<NAME>"), "{error}");
        let error = Destination::parse("KASL_WEBHOOK_team", &format!("slack {HOOK}")).unwrap_err();
        assert!(error.contains("capital"), "{error}");
    }

    #[test]
    fn no_error_repeats_the_credential() {
        // An error is logged, and a log ships to wherever logs go. The failure
        // this guards is the helpful message that quotes the value it could
        // not read - which, here, is a working hook.
        for value in [
            format!("slack {HOOK} stray-SECRETSECRETSECRETSECRET"),
            format!("slack {HOOK} events=SECRETSECRETSECRETSECRET"),
            format!("{HOOK}SECRETSECRETSECRETSECRET slack"),
            format!("slack {HOOK} SECRETSECRETSECRETSECRET=1"),
            "telegram 99:SECRETSECRETSECRETSECRET".to_string(),
            "json https://x.example/SECRETSECRETSECRETSECRET".to_string(),
        ] {
            let error = Destination::parse("KASL_WEBHOOK_TEAM", &value).unwrap_err();
            assert!(!error.contains("SECRETSECRETSECRETSECRET"), "the error quotes the credential: {error}");
        }
    }

    #[test]
    fn what_is_shown_is_the_host_and_never_the_hook() {
        let destination = Destination::parse("KASL_WEBHOOK_TEAM", &format!("slack {HOOK}")).unwrap();
        assert_eq!(destination.shown_target(), "hooks.slack.com");
        assert!(!format!("{destination:?}").contains("SECRET"), "Debug must not print the target");

        assert_eq!(host("https://user:pw@chat.example.com:8443/hooks/x?y=1"), "chat.example.com");
    }

    #[test]
    fn redaction_removes_every_credential_a_target_holds() {
        let destination = Destination::parse("KASL_WEBHOOK_P", "json https://p.example/in secret=s3cr3t-value").unwrap();
        let redacted = destination.redact("POST https://p.example/in failed, signed with s3cr3t-value");
        assert!(!redacted.contains("https://p.example/in"), "{redacted}");
        assert!(!redacted.contains("s3cr3t-value"), "{redacted}");

        let destination = Destination::parse("KASL_WEBHOOK_T", "telegram 42:TOKEN chat=1").unwrap();
        assert_eq!(
            destination.redact("https://api.telegram.org/bot42:TOKEN/sendMessage"),
            "https://api.telegram.org/bot…/sendMessage"
        );
    }
}
