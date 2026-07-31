use crate::framework::{
    ArgIssue, ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, text_result,
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const DEFAULT_MAX_RESULTS: u32 = 100;
const DESCRIPTION: &str = "\
Get recent browser history entries, including URLs, titles, visit times, and visit counts. \
Use maxResults to limit how many entries are returned.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HistoryArgs {
    /// Maximum number of recent entries to return. Defaults to 100.
    #[serde(default = "default_max_results")]
    #[schemars(range(min = 1))]
    max_results: u32,
}

#[derive(Debug, Deserialize)]
struct HistoryResult {
    entries: Vec<HistoryEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    id: String,
    url: String,
    title: String,
    last_visit_time: f64,
    visit_count: i64,
    typed_count: i64,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<HistoryArgs>(
        "history",
        DESCRIPTION,
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: HistoryArgs = parse_args(raw)?;
        validate_args(&args)?;
        let HistoryResult { entries } = serde_json::from_value(
            ctx.session
                .cdp(
                    "History.getRecent",
                    json!({ "maxResults": args.max_results }),
                    None,
                )
                .await?,
        )?;
        let count = entries.len();
        Ok(Some(text_result(
            format_history(&entries),
            Some(json!({ "entries": entries, "count": count })),
        )))
    })
}

fn validate_args(args: &HistoryArgs) -> ToolExecResult<()> {
    if args.max_results == 0 {
        return Err(ToolError::InvalidArguments(vec![ArgIssue {
            path: "maxResults".to_string(),
            message: "Number must be greater than or equal to 1".to_string(),
        }]));
    }
    Ok(())
}

fn format_history(entries: &[HistoryEntry]) -> String {
    if entries.is_empty() {
        return "(no history)".to_string();
    }

    let mut lines = vec![
        format!("Recent history ({}):", entries.len()),
        String::new(),
    ];
    for entry in entries {
        let destination = if entry.title.is_empty() {
            entry.url.clone()
        } else {
            format!("{} ({})", entry.title, entry.url)
        };
        lines.push(format!(
            "- {destination} — last visited {}; {} {}; {} typed",
            format_visit_time(entry.last_visit_time),
            entry.visit_count,
            pluralize(entry.visit_count, "visit", "visits"),
            entry.typed_count,
        ));
    }
    lines.join("\n")
}

fn format_visit_time(milliseconds: f64) -> String {
    if !milliseconds.is_finite() {
        return milliseconds.to_string();
    }
    let Some(nanoseconds) = (milliseconds.trunc() as i128).checked_mul(1_000_000) else {
        return milliseconds.to_string();
    };
    OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
        .ok()
        .and_then(|timestamp| timestamp.format(&Rfc3339).ok())
        .unwrap_or_else(|| milliseconds.to_string())
}

fn pluralize(value: i64, singular: &'static str, plural: &'static str) -> &'static str {
    if value == 1 { singular } else { plural }
}

fn default_max_results() -> u32 {
    DEFAULT_MAX_RESULTS
}
