//! What reaches stdout.
//!
//! The default is the tool's text block, verbatim. That text is already
//! written for an agent to read, refs and all, so reformatting it only loses
//! fidelity. `--json` wraps it without replacing it.

use serde_json::{Value, json};

use crate::cli::client::result::ToolOutput;

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

/// The path a tool reports having written.
///
/// `pdf` and `download` write into the browser's own output directory and name
/// the file in their reply. Both are on this machine, so a caller who asked
/// for a destination can have the file copied there.
#[must_use]
pub fn written_path(output: &ToolOutput) -> Option<String> {
    if let Some(path) = output
        .structured
        .as_ref()
        .and_then(|value| value.get("path"))
        .and_then(Value::as_str)
    {
        return Some(path.to_string());
    }
    output
        .text
        .split(" to: ")
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

/// The tab group a listing reports for one page.
///
/// The group id is the durable half of the session identity: the transport id
/// dies with the server, while a known group can be re-claimed on `tabs new`.
/// It is only discoverable from a listing, as `[group <ID>]` on the page's
/// line, because the server creates the group when the first tab is opened
/// and does not name it in that reply.
#[must_use]
pub fn group_of(output: &ToolOutput, page: u32) -> Option<String> {
    if let Some(group) = output
        .structured
        .as_ref()
        .and_then(|value| value.get("pages"))
        .and_then(Value::as_array)
        .and_then(|pages| {
            pages
                .iter()
                .find(|entry| entry.get("page").and_then(Value::as_u64) == Some(u64::from(page)))
                .and_then(|entry| entry.get("groupId"))
                .and_then(Value::as_str)
        })
    {
        return Some(group.to_string());
    }
    let prefix = format!("[{page}]");
    output
        .text
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with(&prefix))
        .and_then(|line| line.split("[group ").nth(1))
        .and_then(|rest| rest.split(']').next())
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_string)
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
    fn the_written_path_is_read_from_the_tools_own_report() {
        // The real wire shape of `pdf`, which has no output schema and so
        // arrives as text only.
        let output = ToolOutput {
            text: "Saved page 4 as PDF (51204 bytes) to: /home/u/.browseros/tool-output/pdf-1.pdf"
                .into(),
            ..ToolOutput::default()
        };
        assert_eq!(
            written_path(&output).as_deref(),
            Some("/home/u/.browseros/tool-output/pdf-1.pdf")
        );
    }

    #[test]
    fn a_result_that_wrote_nothing_reports_no_path() {
        let output = ToolOutput {
            text: "ok".into(),
            ..ToolOutput::default()
        };
        assert_eq!(written_path(&output), None);
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

    /// The real wire shape, captured live. Without this the scope file never
    /// learns the group, so `session end --close-tabs` has nothing to close
    /// and a restart cannot re-claim the task's group.
    #[test]
    fn the_group_for_a_page_is_read_from_its_listing_line() {
        let output = ToolOutput {
            text: "Your tabs:\n[2] http://x.test/ (CLI check) [group 5D033B84]\n[3] http://y.test/ (Other) [group AABBCC]".into(),
            ..ToolOutput::default()
        };
        assert_eq!(group_of(&output, 2).as_deref(), Some("5D033B84"));
        assert_eq!(group_of(&output, 3).as_deref(), Some("AABBCC"));
        assert_eq!(group_of(&output, 9), None);
    }

    #[test]
    fn an_ungrouped_page_reports_no_group_rather_than_an_empty_one() {
        let output = ToolOutput {
            text: "[2] http://x.test/ (CLI check)".into(),
            ..ToolOutput::default()
        };
        assert_eq!(group_of(&output, 2), None);
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
