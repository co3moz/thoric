//! `GET /api/activity/export`: the filtered log as a file, CSV or JSON lines, newest first. It
//! streams a page of the table at a time, following the pages' cursor, so it holds no more than
//! one page in memory and ends with the kept rows. A row carries what the list answers, its
//! arguments as the log masks them, and nothing a tool returned.
use super::{
    Entry,
    store::{self, Filter, PAGE_LIMIT},
};
use crate::{
    App,
    error::{Error, Result},
};
use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use std::{fmt::Write, sync::Arc};

/// Which calls the file holds, by the list's filters, and its format.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Export {
    /// `csv`, the default, or `jsonl` for one JSON object a line.
    pub format: Option<String>,
    /// A container's ID.
    pub container: Option<String>,
    /// An endpoint's name.
    pub endpoint: Option<String>,
    /// `administrator` for `/mcp`, `endpoint` for every endpoint.
    pub caller: Option<String>,
    /// Part of a tool's or a desktop server's name, in any case.
    pub tool: Option<String>,
    /// `read`, `change` or `destructive`, several separated by commas.
    pub kind: Option<String>,
    /// `ok`, `error` or `refused`, several separated by commas.
    pub outcome: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Csv,
    JsonLines,
}

impl Format {
    fn of(name: Option<&str>) -> Result<Self> {
        match name.unwrap_or("csv") {
            "csv" => Ok(Format::Csv),
            "jsonl" => Ok(Format::JsonLines),
            _ => Err(Error(
                StatusCode::BAD_REQUEST,
                "format is csv or jsonl".into(),
            )),
        }
    }
    fn media(self) -> &'static str {
        match self {
            Format::Csv => "text/csv; charset=utf-8",
            Format::JsonLines => "application/x-ndjson",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::JsonLines => "jsonl",
        }
    }
}

/// The columns of the CSV, in the order [`line`] writes them.
pub(super) const COLUMNS: &str = "id,at,last_at,repeats,endpoint,credential,client,source,server,tool,container_id,container_name,kind,outcome,error,duration_ms,job_id,summary,arguments\n";

/// `GET /api/activity/export`.
pub async fn export(State(app): State<Arc<App>>, Query(export): Query<Export>) -> Result<Response> {
    let format = Format::of(export.format.as_deref())?;
    let filter = Filter {
        container: export.container,
        endpoint: export.endpoint,
        caller: export.caller,
        tool: export.tool,
        kind: export.kind,
        outcome: export.outcome,
        before: None,
        limit: Some(PAGE_LIMIT),
    };
    filter.check()?;
    let name = format!(
        "thoric-activity-{}.{}",
        stamp(super::now_ms() / 1000),
        format.extension()
    );
    // The pages yet to come: the filter with the cursor of the page before, and whether the
    // CSV's heading is still to be written. `None` once a page came back short.
    let pages = futures_util::stream::unfold(Some((app, filter, true)), move |state| async move {
        let (app, mut filter, first) = state?;
        let read = {
            let (app, filter) = (app.clone(), filter.clone());
            tokio::task::spawn_blocking(move || store::query(&app.db, &filter)).await
        };
        let rows = match read {
            Ok(Ok(rows)) => rows,
            // The download breaks off, so a file cut short is not taken for the whole log.
            Ok(Err(error)) => return Some((Err(std::io::Error::other(error)), None)),
            Err(error) => return Some((Err(std::io::Error::other(error)), None)),
        };
        let mut text = String::new();
        if first && format == Format::Csv {
            text.push_str(COLUMNS);
        }
        for row in &rows {
            match format {
                Format::Csv => line(&mut text, row),
                Format::JsonLines => {
                    text.push_str(&serde_json::to_string(row).unwrap_or_default());
                    text.push('\n');
                }
            }
        }
        filter.before = rows.last().map(|row| row.id);
        let next = (rows.len() == PAGE_LIMIT).then_some((app, filter, false));
        Some((Ok(Bytes::from(text)), next))
    });
    Response::builder()
        .header(header::CONTENT_TYPE, format.media())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(pages))
        .map_err(|error| Error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

/// One call as a line of the CSV.
fn line(out: &mut String, row: &Entry) {
    let text = |value: &Option<String>| cell(value.as_deref().unwrap_or_default());
    let fields = [
        row.id.to_string(),
        moment(row.at),
        row.last_at.map(moment).unwrap_or_default(),
        row.repeats.to_string(),
        text(&row.endpoint),
        cell(&row.credential),
        text(&row.client),
        cell(&row.source),
        text(&row.server),
        cell(&row.tool),
        text(&row.container_id),
        text(&row.container_name),
        cell(&row.kind),
        cell(&row.outcome),
        text(&row.error),
        row.duration_ms.to_string(),
        text(&row.job_id),
        cell(&row.summary),
        cell(&row.arguments.to_string()),
    ];
    out.push_str(&fields.join(","));
    out.push('\n');
}

/// A text as a CSV cell: quoted when it holds a comma, a quote or a line break, and behind an
/// apostrophe when it begins as a spreadsheet's formula would, since a script or a path an agent
/// chose must not run as one in the program that opens the file.
fn cell(text: &str) -> String {
    let formula = text.starts_with(['=', '+', '-', '@', '\t', '\r']);
    if !formula && !text.contains([',', '"', '\n', '\r']) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 3);
    out.push('"');
    if formula {
        out.push('\'');
    }
    out.push_str(&text.replace('"', "\"\""));
    out.push('"');
    out
}

/// The year, month and day of `seconds` since 1970 in UTC, by Howard Hinnant's days-to-civil
/// conversion.
fn civil(seconds: u64) -> (i64, i64, i64) {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// A moment in Unix milliseconds as RFC 3339 in UTC, which spreadsheets read as a date.
fn moment(ms: u64) -> String {
    let seconds = ms / 1000;
    let (year, month, day) = civil(seconds);
    let time = seconds % 86_400;
    let mut out = String::new();
    let _ = write!(
        out,
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        time / 3600,
        time / 60 % 60,
        time % 60,
        ms % 1000
    );
    out
}

/// A moment in Unix seconds as a file name carries it, in UTC.
fn stamp(seconds: u64) -> String {
    let (year, month, day) = civil(seconds);
    let time = seconds % 86_400;
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
