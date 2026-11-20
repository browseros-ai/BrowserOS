//! Fitting a tool's text to a shell.
//!
//! The server writes its results for MCP agents, and they keep getting exactly that. What
//! changes here is only the copy the CLI prints: hints that name an MCP tool become commands
//! that run.

use serde_json::Value;

use crate::cli::client::result::ToolOutput;

const PAGE_CONTENT_OPEN: &str = "[UNTRUSTED_PAGE_CONTENT nonce=";

/// Maps each line of `text`, telling `map` whether the line sits inside a page-content fence.
/// The fence lines themselves pass through untouched, and `None` drops a line.
pub fn map_lines(text: &str, mut map: impl FnMut(&str, bool) -> Option<String>) -> String {
    let mut closing: Option<String> = None;
    let mut out = Vec::new();
    for line in text.split('\n') {
        if let Some(close) = &closing {
            if line.trim_end() == close {
                closing = None;
                out.push(line.to_string());
            } else if let Some(mapped) = map(line, true) {
                out.push(mapped);
            }
            continue;
        }
        if let Some(nonce) = line
            .strip_prefix(PAGE_CONTENT_OPEN)
            .and_then(|rest| rest.split([' ', ']']).next())
        {
            closing = Some(format!("[END_UNTRUSTED_PAGE_CONTENT nonce={nonce}]"));
            out.push(line.to_string());
            continue;
        }
        if let Some(mapped) = map(line, false) {
            out.push(mapped);
        }
    }
    out.join("\n")
}

/// The saved output a result names, and the part worth reading from it: the one that is not
/// the `result.json` every saved output carries.
#[must_use]
pub fn saved_output(output: &ToolOutput) -> Option<(String, Option<String>)> {
    let saved = output.structured.as_ref()?.get("output")?;
    let id = saved.get("id")?.as_str()?.to_string();
    let part = saved
        .get("parts")
        .and_then(Value::as_array)
        .and_then(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("name").and_then(Value::as_str))
                .find(|name| *name != "result.json")
        })
        .map(str::to_string);
    Some((id, part))
}

/// The command that reads a saved output back.
#[must_use]
pub fn output_command(saved: Option<&(String, Option<String>)>) -> String {
    match saved {
        Some((id, Some(part))) => format!("browseros output {id} --part {part}"),
        Some((id, None)) => format!("browseros output {id}"),
        None => "browseros output <id>".to_string(),
    }
}

/// Each sentence the server ends a saved-output notice with, and what the CLI says instead.
/// `{cmd}` is the `browseros output` command for this result.
const READ_OUTPUT_HINTS: [(&str, &str); 6] = [
    (
        "Use read_output with outputId and part; omit part to list parts.",
        "Read it with: {cmd}",
    ),
    (
        "Use read_output for the full current snapshot.",
        "Read the full current snapshot with: {cmd}",
    ),
    (
        "Use read_output for the full diff.",
        "Read the full diff with: {cmd}",
    ),
    (
        "Use read_output for the full snapshot and refs.",
        "Read the full snapshot with: {cmd}, or search it with: browseros grep <pattern>",
    ),
    ("use read_output to retrieve them.", "read them with: {cmd}"),
    (
        "use read_output for the available parts.",
        "list them with: {cmd}",
    ),
];

/// Rewrites the server's `read_output` hints into the `browseros output` command that does
/// the same, outside page content only.
#[must_use]
pub fn cli_hints(text: &str, saved: Option<&(String, Option<String>)>) -> String {
    if !text.contains("read_output") {
        return text.to_string();
    }
    let command = output_command(saved);
    map_lines(text, |line, in_page| {
        if in_page || !line.contains("read_output") {
            return Some(line.to_string());
        }
        let mut line = line.to_string();
        for (server, cli) in READ_OUTPUT_HINTS {
            line = line.replace(server, &cli.replace("{cmd}", &command));
        }
        Some(line)
    })
}

/// How core marks a link's address on a snapshot line: after the link's ref, JSON-quoted. A
/// page cannot write it, since the names and values it controls are quoted with escapes.
const URL_OPEN: &str = " [url=\"";
const GREP_TRUNCATED: &str = "... [truncated]";

/// The tools whose text is snapshot lines. A script's return value, a read and a saved part
/// read back on purpose are printed as they are.
#[must_use]
pub fn carries_link_urls(tool: &str) -> bool {
    !matches!(tool, "run" | "evaluate_in_page" | "read" | "read_output")
}

/// Every link address cut to its origin and path. Tracking queries made up most of the bytes of
/// a search results page, three per product, and the path is what tells links apart.
#[must_use]
pub fn short_link_urls(text: &str) -> String {
    if !text.contains(URL_OPEN) {
        return text.to_string();
    }
    map_lines(text, |line, _| Some(short_link_url(line)))
}

fn short_link_url(line: &str) -> String {
    let Some(at) = line.find(URL_OPEN) else {
        return line.to_string();
    };
    if !line[..at].contains("link") {
        return line.to_string();
    }
    let start = at + URL_OPEN.len();
    let rest = &line[start..];
    let mut escaped = false;
    let mut close = None;
    for (index, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => {
                close = Some(index);
                break;
            }
            _ => {}
        }
    }
    let (url, tail) = match close {
        Some(index) => (&rest[..index], rest[index..].to_string()),
        // grep cut the line inside the address, so what followed it is already gone.
        None => {
            let end = rest.find(GREP_TRUNCATED).unwrap_or(rest.len());
            (&rest[..end], format!("\"] {}", &GREP_TRUNCATED[4..]))
        }
    };
    let Some(cut) = url.find(['?', '#']) else {
        return line.to_string();
    };
    format!(
        "{}{}{}...{tail}",
        &line[..start],
        &url[..cut],
        &url[cut..=cut]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn saved() -> (String, Option<String>) {
        ("d0b05dae".to_string(), Some("snapshot.md".to_string()))
    }

    /// Every result sentence the server writes about `read_output`, read from its source so a
    /// new or reworded one fails here rather than reaching an agent as an MCP tool name.
    fn server_hint_literals() -> Vec<String> {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/browseros-mcp/src/"
        );
        let mut literals = Vec::new();
        for file in [
            "framework.rs",
            "output_store.rs",
            "format/diff.rs",
            "format/snapshot.rs",
        ] {
            let source = std::fs::read_to_string(format!("{root}{file}")).unwrap_or_default();
            assert!(!source.is_empty(), "could not read {file}");
            for line in source.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || !line.contains("read_output") {
                    continue;
                }
                if let (Some(start), Some(end)) = (line.find('"'), line.rfind('"'))
                    && end > start
                {
                    literals.push(line[start + 1..end].replace("\\n", "\n"));
                }
            }
        }
        literals
    }

    #[test]
    fn no_server_hint_reaches_the_shell_naming_read_output() {
        let literals = server_hint_literals();
        assert!(literals.len() >= 6, "{literals:?}");
        for literal in literals {
            let rewritten = cli_hints(&literal, Some(&saved()));
            assert!(!rewritten.contains("read_output"), "{rewritten}");
            assert!(rewritten.contains("browseros output"), "{rewritten}");
        }
    }

    #[test]
    fn the_saved_output_line_names_the_part_to_read() {
        let text = "ok\n[Saved output: d0b05dae. Use read_output with outputId and part; omit part to list parts.]";
        assert_eq!(
            cli_hints(text, Some(&saved())),
            "ok\n[Saved output: d0b05dae. Read it with: browseros output d0b05dae --part snapshot.md]"
        );
    }

    #[test]
    fn page_content_that_mentions_read_output_is_left_alone() {
        let text = "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] notice\nUse read_output for the full diff.\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\nUse read_output for the full diff.";
        assert_eq!(
            cli_hints(text, Some(&saved())),
            "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] notice\nUse read_output for the full diff.\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\nRead the full diff with: browseros output d0b05dae --part snapshot.md"
        );
    }

    #[test]
    fn the_part_comes_from_the_structured_result() {
        let output = ToolOutput {
            structured: Some(json!({ "output": { "id": "abc", "parts": [
                { "name": "diff.md" }, { "name": "result.json" }
            ] } })),
            ..ToolOutput::default()
        };
        assert_eq!(
            saved_output(&output),
            Some(("abc".to_string(), Some("diff.md".to_string())))
        );
    }

    #[test]
    fn a_page_content_fence_with_a_different_nonce_does_not_close_it() {
        let text = "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] n\n[END_UNTRUSTED_PAGE_CONTENT nonce=zz]\nin\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\nout";
        let mut seen = Vec::new();
        map_lines(text, |line, in_page| {
            seen.push((line.to_string(), in_page));
            Some(line.to_string())
        });
        assert_eq!(
            seen,
            vec![
                ("[END_UNTRUSTED_PAGE_CONTENT nonce=zz]".to_string(), true),
                ("in".to_string(), true),
                ("out".to_string(), false),
            ]
        );
    }

    const AMAZON_LINE: &str = "  - link \"9,515 ratings\" [ref=e107] [url=\"https://www.amazon.in/sspa/click?ie=UTF8&spc=MTo1OTU1&url=%2FSensodyne-Multi-Pack%2Fdp%2FB0C%3Fpsc%3D1#customerReviews\"]";

    #[test]
    fn a_link_keeps_its_origin_and_path_and_drops_the_query() {
        assert_eq!(
            short_link_urls(AMAZON_LINE),
            "  - link \"9,515 ratings\" [ref=e107] [url=\"https://www.amazon.in/sspa/click?...\"]"
        );
    }

    #[test]
    fn a_link_with_a_value_keeps_it() {
        let line = "- link \"Go\" [ref=e1] [url=\"https://x.test/a?b=c\"]: \"v\"";
        assert_eq!(
            short_link_urls(line),
            "- link \"Go\" [ref=e1] [url=\"https://x.test/a?...\"]: \"v\""
        );
    }

    #[test]
    fn an_address_grep_cut_short_is_closed_and_still_marked_truncated() {
        let line = "- link \"x\" [ref=e3] [url=\"https://a.test/p?q=1234567... [truncated]";
        assert_eq!(
            short_link_urls(line),
            "- link \"x\" [ref=e3] [url=\"https://a.test/p?...\"] [truncated]"
        );
    }

    #[test]
    fn a_link_with_no_query_and_text_that_is_not_a_link_are_left_alone() {
        for line in [
            "- link \"Home\" [ref=e1] [url=\"https://x.test/home\"]",
            "- heading \"About [url=\\\"a?b\\\"]\" [level=2]",
            "price [url=\"https://x.test/?a\"] in visible text",
        ] {
            assert_eq!(short_link_urls(line), line);
        }
    }

    #[test]
    fn a_result_page_of_tracking_links_shrinks_to_a_fraction() {
        let page = AMAZON_LINE.repeat(29);
        let page = page.replace("]  - ", "]\n  - ");
        assert!(short_link_urls(&page).len() * 2 < page.len());
    }
}
