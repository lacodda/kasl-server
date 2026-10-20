---
title: Reading hours into Grafana
description: A read token, the Infinity data source, and three panels - hours against the norm, the days of a month, and who has days missing. Power Query and a plain script at the end.
---

Grafana reads the [BI API](/kasl-server/reference/the-bi-api/) through its
**Infinity** data source, which fetches JSON or CSV over HTTP with a bearer
token. Twenty minutes from a token to a dashboard a manager opens on Monday.

## 1. Issue a token

Sign in as the manager or administrator whose view the dashboard should have -
a token reads exactly what its owner may see. Open **Team**, then **Export →
Read from a dashboard tool…**, name the token after where it is going
(`Grafana`), and **Issue a token**. Copy it now: it is shown once.

A dashboard for the whole company wants an administrator's token. One for a
department wants that department's manager's: it shows their people and nobody
else's, and follows them if the department changes hands.

## 2. Add the data source

1. Install the plugin: **Administration → Plugins**, search for *Infinity*,
   **Install** - or `grafana cli plugins install yesoreyeram-infinity-datasource`
   and a restart.
2. **Connections → Data sources → Add new data source → Infinity**.
3. Under **Authentication**, choose **Bearer Token** and paste the token.
4. Under **Allowed hosts**, add the server's address -
   `https://kasl.example.com`. Infinity refuses to send a credential anywhere
   not listed, which is what keeps the token from leaking to a URL typed by
   mistake.
5. **Save & test**.

## 3. Hours against the norm

A bar chart of the dashboard's range, a bar pair per person.

1. New panel, data source **Infinity**. Type **JSON**, parser **Backend**,
   source **URL**, method **GET**.
2. URL - the range is Grafana's time picker, in the form the server reads:

   ```text
   https://kasl.example.com/api/bi/v1/summary?from=${__from:date:YYYY-MM-DD}&to=${__to:date:YYYY-MM-DD}
   ```

3. **Columns**: `display_name` as *String*, `worked_seconds` and `due_seconds`
   as *Number*.
4. Visualization **Bar chart**, unit **Time → seconds (s)**: Grafana shows
   30600 as *8.50 hour* by itself.

`due_seconds` rather than `norm_seconds` is deliberate: in a range still
running, the norm counts days that have not come yet, and everybody looks
behind on the 10th. `due_seconds` is the part that has come due - what the team
screen measures against too.

## 4. The days of a month

A time series, a line per person.

1. URL: `.../api/bi/v1/days?from=${__from:date:YYYY-MM-DD}&to=${__to:date:YYYY-MM-DD}`
2. **Columns**: `date` as *Timestamp* with format `YYYY-MM-DD`,
   `display_name` as *String*, `worked_seconds` as *Number*.
3. **Transform → Partition by values**, field `display_name`.
4. Visualization **Time series**, unit **seconds (s)**, *Connect null values*
   off: a date with nothing recorded is a gap, and a line drawn across it would
   say otherwise.

`date` is the date in the person's own calendar, as their agent recorded it. A
day in Tokyo and a day in Buenos Aires land on the same date even though they
did not happen at the same time - which is what a chart of working days wants.

## 5. Who has days missing

A table of dates that were owed, have come due, and hold nothing.

1. The same `days` URL, with columns `date`, `display_name`, `department`,
   `state` and `filed`.
2. **Transform → Filter data by values**: include rows where `state` is equal
   to `unrecorded`.
3. Visualization **Table**.

Swap the filter for `filed` equal to `late` and the same panel lists reports
that arrived after the next day had begun. The states and their meanings are in
[the reference](/kasl-server/reference/the-bi-api/#days).

## A department picker

Add a dashboard variable of type **Query** on the Infinity data source, URL
`.../api/bi/v1/departments`, column `department`. Then filter any panel with
**Filter data by values** on `department` equal to `${department}`. The
departments come whole; the people in them are still only the ones the token's
owner may see.

## Power Query, and a script

Excel and Power BI Desktop read the CSV through **Get data → Blank query →
Advanced editor**:

```text
let
    Source = Csv.Document(Web.Contents("https://kasl.example.com/api/bi/v1/summary.csv",
        [Query = [from = "2026-10-01", to = "2026-10-31"],
         Headers = [Authorization = "Bearer kasl_read_..."]])),
    Rows = Table.PromoteHeaders(Source, [PromoteAllScalars = true])
in
    Rows
```

Choose **Anonymous** when it asks how to sign in: the header carries the token.

Anything that can send a header can read it - a Monday report is one line:

```console
$ curl -sf -H "Authorization: Bearer $KASL_READ_TOKEN" \
    "https://kasl.example.com/api/bi/v1/summary.csv?from=2026-10-12&to=2026-10-18" > week.csv
```

## When it stops answering

- **`401`** - the token was revoked, or its owner was deactivated. Issue a new
  one from an account that is meant to outlive the dashboard.
- **`403`** - its owner is no longer a manager. The token reads with their
  access, and they no longer have the team's.
- **`400`** - the range: `to` before `from`, or more than 400 days. A
  dashboard set to *Last 2 years* asks for too much; split it, or narrow it.

The token list says when each one was last read with. One that has not moved in
weeks is pasted somewhere nobody looks - revoke it.
