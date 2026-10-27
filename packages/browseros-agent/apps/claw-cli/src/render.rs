//! What reaches stdout.
//!
//! The default is the tool's text block, verbatim. That text is already
//! written for an agent to read, refs and all, so reformatting it only loses
//! fidelity. `--json` wraps it without replacing it.

use serde_json::{Value, json};

use crate::client::result::ToolOutput;

/// The `--json` envelope.
#[must_use]
pub fn envelope(
    command: &str,
    tool: &str,
    page: Option<u32>,
    output: &ToolOutput,
    elapsed_ms: u128,
    files: &[String],
) -> String {
    let mut value = json!({
        "ok": !output.is_error,
        "command": command,
        "tool": tool,
        "text": output.text,
        "elapsedMs": elapsed_ms,
    });
    if let Some(page) = page {
        value["page"] = json!(page);
    }
    if let Some(structured) = &output.structured {
        value["structured"] = structured.clone();
    }
    if !files.is_empty() {
        value["files"] = json!(files);
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| output.text.clone())
}

/// A key/value block, used by `status` and `session list`.
#[must_use]
pub fn fields(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    rows.iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{key:width$}  {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pulls a page id out of a tool result, for the commands that open one.
#[must_use]
pub fn page_from(output: &ToolOutput) -> Option<u32> {
    output
        .structured
        .as_ref()?
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|page| u32::try_from(page).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelope_keeps_the_text_rather_than_replacing_it() {
        let output = ToolOutput {
            text: "opened page 3".into(),
            ..ToolOutput::default()
        };
        let rendered = envelope("open", "tabs", Some(3), &output, 41, &[]);
        assert!(
            rendered.contains("\"text\": \"opened page 3\""),
            "{rendered}"
        );
        assert!(rendered.contains("\"ok\": true"), "{rendered}");
    }

    #[test]
    fn an_empty_field_is_left_out_rather_than_printed_blank() {
        let block = fields(&[("session", "abc".into()), ("task", String::new())]);
        assert_eq!(block, "session  abc");
    }

    #[test]
    fn the_new_page_id_comes_from_structured_content() {
        let output = ToolOutput {
            structured: Some(json!({ "page": 7 })),
            ..ToolOutput::default()
        };
        assert_eq!(page_from(&output), Some(7));
    }
}
