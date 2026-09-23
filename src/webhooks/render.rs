//! Turning an event into what each kind of destination reads.
//!
//! The wording follows the dashboard's, and for the same reason: every
//! sentence states what was measured and what it was measured against, and
//! none says what it means. A twelve-hour day is a release, a crisis, or kasl
//! left running, and a chat message has no more idea which than the screen
//! does (ADR 0016).
//!
//! Three markups, because the three chats read three: Slack its own `*bold*`
//! and `<url|text>`, Mattermost Markdown, Telegram a small subset of HTML.
//! Each gets its own escaping - a display name is typed by a person, and a
//! name with a `<` in it must arrive as a name, not as markup.

use serde_json::{Value, json};

use super::{Destination, Event, EventKind};
use crate::{alerts::AlertRule, calendar::WorkdayKind};

/// Which markup a chat reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    Slack,
    Markdown,
    TelegramHtml,
}

impl Markup {
    fn escape(self, text: &str) -> String {
        match self {
            // Slack's own rule: exactly these three, and nothing else.
            Self::Slack => text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
            Self::Markdown => {
                let mut escaped = String::with_capacity(text.len());
                for c in text.chars() {
                    if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '(' | ')' | '~' | '#' | '>' | '|' | '!') {
                        escaped.push('\\');
                    }
                    escaped.push(c);
                }
                escaped
            }
            Self::TelegramHtml => text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;"),
        }
    }

    fn bold(self, text: &str) -> String {
        let text = self.escape(text);
        match self {
            Self::Slack => format!("*{text}*"),
            Self::Markdown => format!("**{text}**"),
            Self::TelegramHtml => format!("<b>{text}</b>"),
        }
    }

    fn link(self, url: &str, label: &str) -> String {
        match self {
            Self::Slack => format!("<{url}|{}>", self.escape(label)),
            Self::Markdown => format!("[{}]({url})", self.escape(label)),
            Self::TelegramHtml => format!("<a href=\"{}\">{}</a>", self.escape(url), self.escape(label)),
        }
    }
}

/// The message a chat shows for an event.
///
/// A headline naming who it is about, the sentence, and the link when the
/// installation knows its address. `destination` is there for the test
/// message, which says what that channel will hear.
pub fn text(event: &Event, destination: &Destination, markup: Markup) -> String {
    let who = event.person.as_ref().map(|person| match &person.department {
        Some(department) => format!("{} ({department})", person.name),
        None => person.name.clone(),
    });

    let mut lines = Vec::new();
    match event.event {
        EventKind::AlertRaised => {
            lines.push(format!(
                "{} {}",
                markup.escape("Needs attention:"),
                markup.bold(who.as_deref().unwrap_or("somebody"))
            ));
        }
        EventKind::AlertAcknowledged => {
            let by = event.by.as_deref().unwrap_or("Somebody");
            lines.push(format!(
                "{} {}",
                markup.escape(&format!("{by} looked at it:")),
                markup.bold(who.as_deref().unwrap_or("somebody"))
            ));
        }
        EventKind::AlertResolved => {
            lines.push(format!("{} {}", markup.escape("Resolved:"), markup.bold(who.as_deref().unwrap_or("somebody"))));
        }
        EventKind::DayClosed => {
            let date = event.day.as_ref().map(|day| day.date.to_string()).unwrap_or_default();
            lines.push(format!(
                "{} {}",
                markup.bold(who.as_deref().unwrap_or("Somebody")),
                markup.escape(&format!("closed {date}"))
            ));
        }
        EventKind::Test => {
            lines.push(markup.bold(&format!("Test message from kasl-server {}", event.server_version)));
        }
    }

    let sentence = match event.event {
        EventKind::AlertRaised | EventKind::AlertAcknowledged => event.alert.as_ref().map(alert_sentence),
        // Resolution states what it was, not a new measurement: the figures
        // are the ones it fired on, and "their agent is back" is the whole
        // news.
        EventKind::AlertResolved => event.alert.as_ref().map(|alert| format!("Was: {}", lowercase_first(&alert_sentence(alert)))),
        EventKind::DayClosed => event.day.as_ref().map(|day| match day.kind {
            WorkdayKind::Work => format!("{} h worked", hours(day.worked_seconds)),
            WorkdayKind::Vacation => "Marked as vacation".to_string(),
            WorkdayKind::Sick => "Marked as sick".to_string(),
            WorkdayKind::DayOff => "Marked as a day off".to_string(),
        }),
        EventKind::Test => {
            let heard: Vec<&str> = destination.events.iter().map(|event| event.name()).collect();
            let about = match &destination.department {
                Some(department) => format!(" about people in {department}"),
                None => String::new(),
            };
            Some(format!("This channel hears {}{about}.", heard.join(", ")))
        }
    };
    if let Some(sentence) = sentence {
        lines.push(markup.escape(&sentence));
    }

    if let Some(url) = &event.link {
        lines.push(markup.link(url, "Open in kasl-server"));
    }

    lines.join("\n")
}

/// What an alert says, in the dashboard's words.
fn alert_sentence(alert: &super::AlertPayload) -> String {
    let date = alert.subject_date.map(|date| date.to_string()).unwrap_or_default();
    match alert.rule {
        AlertRule::NoAgentData => format!("Nothing from their agent for {}", span(alert.observed_seconds)),
        AlertRule::Overwork => format!(
            "Worked {} h on {date}, against a norm of {} h",
            hours(alert.observed_seconds),
            alert.against_seconds.map(hours).unwrap_or_else(|| "—".to_string())
        ),
        AlertRule::DayNotClosed => format!("The day of {date} has been open for {}", span(alert.observed_seconds)),
    }
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The body a Slack or Mattermost incoming webhook takes.
pub fn hook_body(text: String) -> Value {
    json!({ "text": text })
}

/// The body Telegram's `sendMessage` takes.
pub fn telegram_body(chat: &str, text: String) -> Value {
    json!({
        "chat_id": chat,
        "text": text,
        "parse_mode": "HTML",
        // The link is to a dashboard behind a login; a preview card would show
        // the sign-in page and nothing else.
        "link_preview_options": { "is_disabled": true },
    })
}

/// Seconds as a bare number of hours: `8`, `12.6`. The dashboard's shape, so
/// a figure reads the same in the channel as on the screen.
pub fn hours(seconds: i64) -> String {
    let tenths = (seconds as f64 / 360.0).round() as i64;
    if tenths % 10 == 0 {
        format!("{}", tenths / 10)
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// A stretch of elapsed time in the largest unit it fills: `9 h`, `2 d`,
/// `13 mo`, `1 y`. For silence and open days, which have no upper bound -
/// never for hours worked, which sit beside a norm in hours.
///
/// The same steps as the dashboard's `span`: one divisor per unit, checked
/// from the largest down, so every value lands in exactly one.
pub fn span(seconds: i64) -> String {
    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;

    if seconds >= YEAR {
        format!("{} y", seconds / YEAR)
    } else if seconds >= MONTH {
        format!("{} mo", seconds / MONTH)
    } else if seconds >= 2 * DAY {
        format!("{} d", seconds / DAY)
    } else {
        format!("{} h", hours(seconds))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone, Utc};
    use uuid::Uuid;

    use super::*;
    use crate::webhooks::{AlertPayload, DayPayload, Person, Webhooks};

    fn destination() -> Destination {
        Destination::parse("KASL_WEBHOOK_TEAM", "slack https://hooks.slack.com/services/T/B/X").unwrap()
    }

    fn person(name: &str) -> Person {
        Person {
            id: Uuid::nil(),
            name: name.to_string(),
            department: Some("Design".to_string()),
        }
    }

    fn silence(seconds: i64) -> AlertPayload {
        AlertPayload {
            id: Uuid::nil(),
            rule: AlertRule::NoAgentData,
            observed_seconds: seconds,
            against_seconds: Some(12 * 3600),
            subject_date: None,
            fired_at: Utc.with_ymd_and_hms(2026, 9, 22, 10, 0, 0).unwrap(),
        }
    }

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 10, 0, 0).unwrap()
    }

    #[test]
    fn an_alert_reads_like_the_dashboard() {
        let webhooks = Webhooks::new(vec![], Some("https://kasl.example.com"));
        let event = Event::alert(EventKind::AlertRaised, silence(13 * 3600), person("Ana Ruiz"), None, &webhooks, now());
        assert_eq!(
            text(&event, &destination(), Markup::Slack),
            "Needs attention: *Ana Ruiz (Design)*\nNothing from their agent for 13 h\n<https://kasl.example.com/team/00000000-0000-0000-0000-000000000000|Open in kasl-server>"
        );
    }

    #[test]
    fn resolution_says_what_it_was_and_acknowledgement_says_who() {
        let webhooks = Webhooks::default();
        let resolved = Event::alert(EventKind::AlertResolved, silence(30 * 3600), person("Ana"), None, &webhooks, now());
        assert_eq!(
            text(&resolved, &destination(), Markup::Markdown),
            "Resolved: **Ana \\(Design\\)**\nWas: nothing from their agent for 30 h"
        );

        let answered = Event::alert(
            EventKind::AlertAcknowledged,
            silence(13 * 3600),
            person("Ana"),
            Some("Lena".into()),
            &webhooks,
            now(),
        );
        assert!(text(&answered, &destination(), Markup::Slack).starts_with("Lena looked at it: *Ana (Design)*"));
    }

    #[test]
    fn a_name_arrives_as_a_name_and_not_as_markup() {
        let webhooks = Webhooks::default();
        let event = Event::alert(EventKind::AlertRaised, silence(13 * 3600), person("<b>Bob</b> & *Co*"), None, &webhooks, now());

        let slack = text(&event, &destination(), Markup::Slack);
        assert!(slack.contains("&lt;b&gt;Bob&lt;/b&gt; &amp; *Co*"), "{slack}");

        let telegram = text(&event, &destination(), Markup::TelegramHtml);
        assert!(telegram.contains("<b>&lt;b&gt;Bob&lt;/b&gt; &amp; *Co* (Design)</b>"), "{telegram}");

        let markdown = text(&event, &destination(), Markup::Markdown);
        assert!(markdown.contains(r"**<b\>Bob</b\> & \*Co\* \(Design\)**"), "{markdown}");
    }

    #[test]
    fn a_closed_day_says_its_hours_or_its_kind() {
        let webhooks = Webhooks::default();
        let date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let day = |kind, worked| DayPayload {
            date,
            kind,
            started_at: now(),
            ended_at: now(),
            worked_seconds: worked,
        };
        let worked = Event::day_closed(Uuid::nil(), day(WorkdayKind::Work, 8 * 3600 + 1800), person("Ana"), &webhooks, now());
        assert_eq!(text(&worked, &destination(), Markup::Slack), "*Ana (Design)* closed 2026-09-21\n8.5 h worked");

        let away = Event::day_closed(Uuid::nil(), day(WorkdayKind::Sick, 0), person("Ana"), &webhooks, now());
        assert!(text(&away, &destination(), Markup::Slack).ends_with("Marked as sick"));
    }

    #[test]
    fn a_test_says_what_the_channel_will_hear() {
        let event = Event::test(&Webhooks::default(), now());
        let message = text(&event, &destination(), Markup::Slack);
        assert!(
            message.contains("This channel hears alert.raised, alert.acknowledged, alert.resolved."),
            "{message}"
        );
    }

    #[test]
    fn spans_step_without_a_gap() {
        // The dashboard's steps, checked by walking every hour of four
        // hundred days: steps on different divisors leave values that belong
        // to neither, and only a sweep finds them.
        const UNITS: [&str; 4] = ["h", "d", "mo", "y"];
        let mut previous = 0;
        for hour in 0..(400 * 24) {
            let shown = span(hour * 3600);
            let unit = shown.rsplit(' ').next().unwrap();
            let order = UNITS.iter().position(|u| *u == unit).unwrap_or_else(|| panic!("an unknown unit: {shown}"));
            assert!(order >= previous, "the unit went back at hour {hour}: {shown}");
            previous = order;
        }
        assert_eq!(span(47 * 3600), "47 h");
        assert_eq!(span(48 * 3600), "2 d");
        assert_eq!(span(9460 * 3600 + 2520), "1 y");
        assert_eq!(hours(12 * 3600 + 2160), "12.6");
        assert_eq!(hours(8 * 3600), "8");
    }
}
