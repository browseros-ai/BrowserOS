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

/// The page ids a `tabs action=list` result names.
///
/// Read from the text, for the same reason [`page_from`] is: `dispatch.rs`
/// strips `structuredContent` from every tool without an output schema, so the
/// listing arrives as lines of `[id] url (title)` and nothing else. Reading
/// structured content here meant every listing looked empty, which made the
/// stale-page explainer clear the remembered page after any tool failure.
#[must_use]
pub fn listed_pages(output: &ToolOutput) -> Vec<u32> {
    if let Some(pages) = output
        .structured
        .as_ref()
        .and_then(|value| value.get("pages"))
        .and_then(Value::as_array)
    {
        return pages
            .iter()
            .filter_map(|entry| entry.get("page").and_then(Value::as_u64))
            .filter_map(|page| u32::try_from(page).ok())
            .collect();
    }
    output
        .text
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('[')?;
            let (id, _) = rest.split_once(']')?;
            id.parse().ok()
        })
        .collect()
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
///
/// The text block is the primary source, not a fallback. `dispatch.rs` strips
/// `structuredContent` from every tool that has no `output_schema`, which is
/// all of them except `run`, so reading structured content alone meant `open`
/// never remembered the page it had just opened. Found against a live server;
/// a fixture that supplies structured content cannot catch it.
#[must_use]
pub fn page_from(output: &ToolOutput) -> Option<u32> {
    structured_page(output).or_else(|| text_page(&output.text))
}

fn structured_page(output: &ToolOutput) -> Option<u32> {
    output
        .structured
        .as_ref()?
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|page| u32::try_from(page).ok())
}

/// `tabs action=new` says `opened page N`. A group advisory may precede it, and
/// an auto-included snapshot follows it, so this scans rather than anchoring.
fn text_page(text: &str) -> Option<u32> {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .windows(3)
        .find(|window| window[0] == "opened" && window[1] == "page")
        .and_then(|window| window[2].trim_end_matches(['.', ',']).parse().ok())
}

/// The url of the page a result describes, where the result says.
///
/// Two shapes carry it: `navigate` reports `navigated (url) -> <url>`, and any
/// result with an inlined snapshot carries `origin=<url>` in the trust-boundary
/// marker. Recording it is what lets `status` report a recycled page id rather
/// than acting on one.
#[must_use]
pub fn url_from(output: &ToolOutput) -> Option<String> {
    if let Some(url) = output
        .structured
        .as_ref()
        .and_then(|value| value.get("url"))
        .and_then(Value::as_str)
    {
        return Some(url.to_string());
    }
    if let Some(rest) = output.text.split(" -> ").nth(1) {
        let url = rest.lines().next().unwrap_or_default().trim();
        if !url.is_empty() && url != "unknown" {
            return Some(url.to_string());
        }
    }
    output
        .text
        .split("origin=")
        .nth(1)
        .and_then(|rest| rest.split([']', ' ']).next())
        .map(str::to_string)
        .filter(|url| !url.is_empty())
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
    fn the_new_page_id_comes_from_structured_content_when_a_tool_sends_it() {
        let output = ToolOutput {
            structured: Some(json!({ "page": 7 })),
            ..ToolOutput::default()
        };
        assert_eq!(page_from(&output), Some(7));
    }

    /// The real wire shape of `tabs action=new`, captured from a live server:
    /// no structured content, because dispatch strips it from every tool
    /// without an output schema.
    #[test]
    fn the_new_page_id_is_read_from_the_text_when_structured_content_is_stripped() {
        let output = ToolOutput {
            text: "opened page 1\n\n--- Additional context (auto-included) ---\n[Page 1 snapshot]\n[UNTRUSTED_PAGE_CONTENT nonce=58d6 origin=http://127.0.0.1:8744/] Untrusted page content follows.\n- heading \"Parts\" [level=1]".into(),
            ..ToolOutput::default()
        };
        assert_eq!(page_from(&output), Some(1));
        assert_eq!(url_from(&output).as_deref(), Some("http://127.0.0.1:8744/"));
    }

    #[test]
    fn a_group_advisory_before_the_page_line_does_not_hide_it() {
        let output = ToolOutput {
            text: "the group id you passed is not open, so the page was not added to it\nopened page 12".into(),
            ..ToolOutput::default()
        };
        assert_eq!(page_from(&output), Some(12));
    }

    #[test]
    fn navigate_reports_its_destination_url() {
        let output = ToolOutput {
            text: "navigated (url) -> https://example.test/a?b=c".into(),
            ..ToolOutput::default()
        };
        assert_eq!(
            url_from(&output).as_deref(),
            Some("https://example.test/a?b=c")
        );
    }

    #[test]
    fn an_unknown_destination_is_not_recorded_as_a_url() {
        let output = ToolOutput {
            text: "navigated (reload) -> unknown".into(),
            ..ToolOutput::default()
        };
        assert_eq!(url_from(&output), None);
    }

    /// The real wire shape of `tabs action=list`, captured from a live server.
    #[test]
    fn a_listing_is_read_from_its_lines_when_structured_content_is_stripped() {
        let output = ToolOutput {
            text: "[1] chrome://newtab/ (New Tab)\n[4] http://127.0.0.1:8744/ (CLI check)".into(),
            ..ToolOutput::default()
        };
        assert_eq!(listed_pages(&output), vec![1, 4]);
    }

    #[test]
    fn an_empty_listing_is_distinguishable_from_an_unreadable_one() {
        let empty = ToolOutput {
            text: "(no open pages)".into(),
            ..ToolOutput::default()
        };
        assert!(listed_pages(&empty).is_empty());
    }

    #[test]
    fn a_listing_that_does_send_structured_content_is_still_preferred() {
        let output = ToolOutput {
            text: "[9] ignored".into(),
            structured: Some(json!({ "pages": [{ "page": 1 }, { "page": 2 }] })),
            ..ToolOutput::default()
        };
        assert_eq!(listed_pages(&output), vec![1, 2]);
    }

    #[test]
    fn text_with_no_page_line_yields_nothing_rather_than_a_guess() {
        let output = ToolOutput {
            text: "closed page 3".into(),
            ..ToolOutput::default()
        };
        assert_eq!(page_from(&output), None);
    }
}
