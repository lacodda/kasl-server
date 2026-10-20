//! A BI answer as a table: one list of columns, written as JSON or as CSV.
//!
//! The two encodings are two spellings of one table, so they are built from it
//! rather than beside it. A row is an array as wide as the column list - a row
//! with a column too few does not compile - and each value is typed by what it
//! means, so a date or a duration is written the same way in both.

use axum::{
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rust_decimal::Decimal;
use serde::{Serialize, Serializer, ser::SerializeMap, ser::SerializeSeq};
use uuid::Uuid;

use crate::export::{defuse, push_csv_line};

/// One value of a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    /// A count, or a duration in whole seconds.
    Int(i64),
    /// A share of a full day.
    Share(Decimal),
    Date(NaiveDate),
    /// A moment, written in UTC to the second.
    Instant(DateTime<Utc>),
    Flag(bool),
    Id(Uuid),
    /// Nothing to say: an open day's total, a date with no record. `null` in
    /// JSON and an empty field in CSV - never a zero (ADR 0015).
    Null,
}

impl Value {
    /// Anything optional, as a value or as nothing.
    pub fn or_null<T>(value: Option<T>, into: impl FnOnce(T) -> Value) -> Value {
        value.map_or(Value::Null, into)
    }

    /// One of the API's own names for a state, a kind or a role - the word
    /// serde gives it, so the BI vocabulary is the API's and cannot drift from
    /// it.
    pub fn word(value: impl Serialize) -> Value {
        match serde_json::to_value(value) {
            Ok(serde_json::Value::String(word)) => Value::Text(word),
            other => unreachable!("an API name serializes to a string, got {other:?}"),
        }
    }

    fn instant_text(instant: &DateTime<Utc>) -> String {
        instant.to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    fn csv(&self) -> String {
        match self {
            // Text typed by people is defused the way the exports defuse it: a
            // CSV is opened in a spreadsheet as often as it is parsed.
            Value::Text(text) => defuse(text),
            Value::Int(number) => number.to_string(),
            // Spelled as the JSON spells it - `1.0`, `0.5` - so the two
            // encodings never disagree about a value.
            Value::Share(share) => serde_json::to_string(share).expect("a decimal always serializes"),
            Value::Date(date) => date.to_string(),
            Value::Instant(instant) => Self::instant_text(instant),
            Value::Flag(flag) => flag.to_string(),
            Value::Id(id) => id.to_string(),
            Value::Null => String::new(),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Text(text) => serializer.serialize_str(text),
            Value::Int(number) => serializer.serialize_i64(*number),
            Value::Share(share) => Serialize::serialize(share, serializer),
            Value::Date(date) => date.serialize(serializer),
            Value::Instant(instant) => serializer.serialize_str(&Self::instant_text(instant)),
            Value::Flag(flag) => serializer.serialize_bool(*flag),
            Value::Id(id) => id.serialize(serializer),
            Value::Null => serializer.serialize_none(),
        }
    }
}

/// A dataset: its columns, and rows exactly as wide.
pub struct Table<const N: usize> {
    pub columns: &'static [&'static str; N],
    pub rows: Vec<[Value; N]>,
}

impl<const N: usize> Table<N> {
    pub fn new(columns: &'static [&'static str; N]) -> Self {
        Self { columns, rows: Vec::new() }
    }

    /// An array of objects, keys in the columns' order.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a table of plain values always serializes")
    }

    /// RFC 4180, like the exports: UTF-8, CRLF, a header row, no byte-order
    /// mark.
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        let header: Vec<String> = self.columns.iter().map(|column| (*column).to_string()).collect();
        push_csv_line(&mut out, &header);
        for row in &self.rows {
            let fields: Vec<String> = row.iter().map(Value::csv).collect();
            push_csv_line(&mut out, &fields);
        }
        out
    }

    /// The table as a JSON response.
    pub fn json(&self) -> Response {
        answer(self.to_json(), "application/json")
    }

    /// The table as a CSV response.
    pub fn csv(&self) -> Response {
        answer(self.to_csv().into_bytes(), "text/csv; charset=utf-8")
    }
}

impl<const N: usize> Serialize for Table<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows = serializer.serialize_seq(Some(self.rows.len()))?;
        for row in &self.rows {
            rows.serialize_element(&Row {
                columns: self.columns,
                values: row,
            })?;
        }
        rows.end()
    }
}

/// One row as an object. A map written field by field rather than a
/// `serde_json::Map`, which would sort the keys and lose the columns' order.
struct Row<'a, const N: usize> {
    columns: &'static [&'static str; N],
    values: &'a [Value; N],
}

impl<const N: usize> Serialize for Row<'_, N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(N))?;
        for (column, value) in self.columns.iter().zip(self.values) {
            map.serialize_entry(column, value)?;
        }
        map.end()
    }
}

/// A body read by a program, not saved by a browser - inline rather than an
/// attachment - and kept out of every cache on the way: it is people's hours.
fn answer(body: Vec<u8>, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: &[&str; 4] = &["name", "worked_seconds", "date", "note"];

    fn table() -> Table<4> {
        let mut table = Table::new(COLUMNS);
        table.rows.push([
            Value::Text("=SUM(A1)".into()),
            Value::Int(27_000),
            Value::Date("2026-10-19".parse().unwrap()),
            Value::Null,
        ]);
        table
    }

    #[test]
    fn json_keeps_the_columns_in_their_order() {
        // `serde_json::Value` would sort the keys; a reader comparing the JSON
        // with the CSV header must find the same order in both.
        let json = String::from_utf8(table().to_json()).unwrap();
        assert_eq!(json, r#"[{"name":"=SUM(A1)","worked_seconds":27000,"date":"2026-10-19","note":null}]"#);
    }

    #[test]
    fn csv_has_the_same_columns_and_defuses_what_people_typed() {
        let csv = table().to_csv();
        assert_eq!(csv, "name,worked_seconds,date,note\r\n'=SUM(A1),27000,2026-10-19,\r\n");
    }

    #[test]
    fn an_instant_is_utc_to_the_second_in_both() {
        let instant: DateTime<Utc> = "2026-10-19T09:15:30.250+03:00".parse().unwrap();
        assert_eq!(Value::Instant(instant).csv(), "2026-10-19T06:15:30Z");
        assert_eq!(serde_json::to_string(&Value::Instant(instant)).unwrap(), r#""2026-10-19T06:15:30Z""#);
    }

    #[test]
    fn a_share_is_spelled_the_same_in_json_and_csv() {
        for (share, spelled) in [(Decimal::new(500, 3), "0.5"), (Decimal::ONE, "1.0"), (Decimal::new(875, 3), "0.875")] {
            assert_eq!(serde_json::to_string(&Value::Share(share)).unwrap(), spelled);
            assert_eq!(Value::Share(share).csv(), spelled);
        }
    }

    #[test]
    fn a_word_is_the_api_name() {
        assert_eq!(Value::word(crate::calendar::WorkdayKind::DayOff), Value::Text("day_off".into()));
    }
}
