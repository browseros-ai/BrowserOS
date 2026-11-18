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
/// From the structured result when the reply has one, which it does on current servers,
/// else from the `[id] url (title)` lines of its text. Servers before #3134 stripped the
/// structured result from every tool without an output schema, and reading it alone made
/// every listing look empty there, so the stale-page explainer cleared the page on any failure.
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

/// The line a navigating tool's result puts before the snapshot it adds on its own.
const AUTO_SNAPSHOT_MARKER: &str = "--- Additional context (auto-included) ---";

/// The most of an auto-included snapshot printed after `open` or `goto`, about 2,000 tokens.
/// Its job is to save a follow-up call on a small page; on a large one it is thousands of
/// tokens the caller did not ask for.
pub const AUTO_SNAPSHOT_MAX_CHARS: usize = 8_000;

/// A navigating result with a large auto-included snapshot cut to one line saying how to see
/// it. A small one is left as it is. The server's own budget stays as MCP agents have it.
#[must_use]
pub fn summarize_auto_snapshot(text: &str, max_chars: usize) -> String {
    let Some((head, context)) = text.split_once(AUTO_SNAPSHOT_MARKER) else {
        return text.to_string();
    };
    if context.chars().count() <= max_chars {
        return text.to_string();
    }
    let saved = context
        .lines()
        .find_map(|line| line.split_once("saved to: ").map(|(_, path)| path.trim()))
        .map(|path| format!(" The full snapshot is saved at {path}."))
        .unwrap_or_default();
    format!(
        "{}\n[the page snapshot is large ({} chars), so it is left out. Run `browseros snapshot -i` to list the page's interactive elements, or `browseros read` for its text.{saved}]",
        head.trim_end(),
        context.chars().count()
    )
}

/// Every listed page that sits in `group`, for leaving a task's pages open as
/// its group goes.
#[must_use]
pub fn pages_in_group(output: &ToolOutput, group: &str) -> Vec<u32> {
    listed_pages(output)
        .into_iter()
        .filter(|page| group_of(output, *page).as_deref() == Some(group))
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

/// The tab group a listing reports for one page, as `[group <ID>]` on the
/// page's line. The server names the task's own group in every result, so this
/// is for the other groups a listing shows, such as which pages are in one.
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

/// A key/value block, used by `status` and `task start`.
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
/// From the structured result when the reply has one, which it does on current servers,
/// else from the text. Servers before #3134 stripped the structured result from every
/// tool without an output schema, and reading it alone meant `open` never remembered the
/// page it had just opened; the text fallback keeps such a server working.
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
        let block = fields(&[("group", "abc".into()), ("task", String::new())]);
        assert_eq!(block, "group  abc");
    }

    #[test]
    fn the_new_page_id_comes_from_structured_content_when_a_tool_sends_it() {
        let output = ToolOutput {
            structured: Some(json!({ "page": 7 })),
            ..ToolOutput::default()
        };
        assert_eq!(page_from(&output), Some(7));
    }

    /// The wire shape of `tabs action=new` from a server that strips the structured result,
    /// captured live before #3134. The text fallback still has to read it.
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

    /// The real wire shape, captured live. `task done --keep-tabs` reads which
    /// pages are in the task's group from it.
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

    #[test]
    fn a_large_auto_snapshot_becomes_one_line() {
        let big = format!(
            "opened page 4\n{AUTO_SNAPSHOT_MARKER}\nLarge snapshot (68947 estimated tokens, 275788 chars) saved to: /tmp/out/snapshot.txt\n{}",
            "- link \"x\" [ref=e1]\n".repeat(600)
        );
        let summary = summarize_auto_snapshot(&big, AUTO_SNAPSHOT_MAX_CHARS);
        assert!(summary.starts_with("opened page 4\n[the page snapshot is large"));
        assert!(summary.contains("browseros snapshot -i"));
        assert!(summary.contains("/tmp/out/snapshot.txt"));
        assert!(!summary.contains("[ref=e1]"));
    }

    #[test]
    fn a_small_auto_snapshot_is_printed_as_it_is() {
        let small = format!("opened page 4\n{AUTO_SNAPSHOT_MARKER}\n- button \"Go\" [ref=e1]");
        assert_eq!(
            summarize_auto_snapshot(&small, AUTO_SNAPSHOT_MAX_CHARS),
            small
        );
    }
}
