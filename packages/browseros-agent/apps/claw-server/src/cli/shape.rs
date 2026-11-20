//! Fitting a tool's text to a shell.
//!
//! The server writes its results for MCP agents, and they keep getting exactly that. What
//! changes here is only the copy the CLI prints: hints that name an MCP tool become commands
//! that run.

use serde_json::Value;

use crate::cli::client::result::ToolOutput;

const PAGE_CONTENT_OPEN: &str = "[UNTRUSTED_PAGE_CONTENT nonce=";

/// Where a line of tool text sits relative to the page-content fences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Fence,
    Page,
    Outside,
}

fn placed_lines(text: &str) -> Vec<(&str, Place)> {
    let mut closing: Option<String> = None;
    let mut placed = Vec::new();
    for line in text.split('\n') {
        if let Some(close) = &closing {
            if line.trim_end() == close {
                closing = None;
                placed.push((line, Place::Fence));
            } else {
                placed.push((line, Place::Page));
            }
            continue;
        }
        if let Some(nonce) = line
            .strip_prefix(PAGE_CONTENT_OPEN)
            .and_then(|rest| rest.split([' ', ']']).next())
        {
            closing = Some(format!("[END_UNTRUSTED_PAGE_CONTENT nonce={nonce}]"));
            placed.push((line, Place::Fence));
            continue;
        }
        placed.push((line, Place::Outside));
    }
    placed
}

/// Maps each line of `text`, telling `map` whether the line sits inside a page-content fence.
/// The fence lines themselves pass through untouched, and `None` drops a line.
pub fn map_lines(text: &str, mut map: impl FnMut(&str, bool) -> Option<String>) -> String {
    placed_lines(text)
        .into_iter()
        .filter_map(|(line, place)| match place {
            Place::Fence => Some(line.to_string()),
            Place::Page => map(line, true),
            Place::Outside => map(line, false),
        })
        .collect::<Vec<_>>()
        .join("\n")
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

/// What an action's one-line summary tells the caller to do next, and the command for it.
const SNAPSHOT_HINTS: [(&str, &str); 2] = [
    (
        "Take a snapshot for the current state.",
        "Run `browseros snapshot -i` for the current state.",
    ),
    (
        "Take a snapshot to see details.",
        "Run `browseros snapshot -i` to see the page now.",
    ),
];

/// Rewrites the server's hints that name an MCP tool or step into the `browseros` command that
/// does the same, outside page content only.
#[must_use]
pub fn cli_hints(text: &str, saved: Option<&(String, Option<String>)>) -> String {
    let names_mcp = |line: &str| line.contains("read_output") || line.contains("Take a snapshot");
    if !names_mcp(text) {
        return text.to_string();
    }
    let command = output_command(saved);
    map_lines(text, |line, in_page| {
        if in_page || !names_mcp(line) {
            return Some(line.to_string());
        }
        let mut line = line.to_string();
        for (server, cli) in READ_OUTPUT_HINTS {
            line = line.replace(server, &cli.replace("{cmd}", &command));
        }
        for (server, cli) in SNAPSHOT_HINTS {
            line = line.replace(server, cli);
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

/// Table structure that `snapshot -i` keeps only because a ref sits under it. A date picker is
/// a grid of rows and cells around one button per day, and those lines filled the list before
/// the dialog's own buttons could appear.
const STRUCTURE_ROLES: [&str; 2] = ["row", "gridcell"];

/// The most of an interactive snapshot printed whole, about the 5,000 tokens the server shows
/// inline. Past it the CLI prints the head and the tail, where a dialog's buttons sit.
pub const INTERACTIVE_INLINE_MAX_CHARS: usize = 20_000;
const INTERACTIVE_HEAD_CHARS: usize = 14_000;

struct SnapshotLine<'a> {
    text: &'a str,
    in_page: bool,
    indent: usize,
    role: &'a str,
    name: Option<&'a str>,
    has_ref: bool,
}

fn snapshot_line(text: &str, in_page: bool) -> SnapshotLine<'_> {
    let body = text.trim_start();
    let indent = text.len() - body.len();
    let item = body.strip_prefix("- ").unwrap_or("");
    let role = item.split([' ', ':']).next().unwrap_or("");
    let rest = &item[role.len()..];
    let (name, after_name) = match rest.strip_prefix(" \"") {
        Some(quoted) => match closing_quote(quoted) {
            Some(end) => (Some(&quoted[..end]), &quoted[end + 1..]),
            None => (None, rest),
        },
        None => (None, rest),
    };
    SnapshotLine {
        text,
        in_page,
        indent,
        role,
        name,
        has_ref: after_name.contains("[ref="),
    }
}

fn closing_quote(quoted: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, ch) in quoted.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(index);
        }
    }
    None
}

/// An interactive snapshot without the rows and cells that only frame its refs: a row or cell
/// with no ref of its own, and a cell whose only child is a ref with the same name, which listed
/// every date twice.
#[must_use]
pub fn interactive_lines(text: &str) -> String {
    let lines = placed_lines(text)
        .into_iter()
        .map(|(line, place)| (place, snapshot_line(line, place == Place::Page)))
        .collect::<Vec<_>>();
    lines
        .iter()
        .enumerate()
        .filter(|(index, (place, line))| {
            *place != Place::Page || !frames_only(line, lines[index + 1..].iter().map(|(_, l)| l))
        })
        .map(|(_, (_, line))| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn frames_only<'a>(
    line: &SnapshotLine<'_>,
    after: impl Iterator<Item = &'a SnapshotLine<'a>>,
) -> bool {
    if !STRUCTURE_ROLES.contains(&line.role) {
        return false;
    }
    if !line.has_ref {
        return true;
    }
    let children = after
        .take_while(|child| child.in_page && child.indent > line.indent)
        .collect::<Vec<_>>();
    matches!(children.as_slice(), [child] if child.has_ref && child.name.is_some() && child.name == line.name)
}

/// A whole interactive snapshot fitted to the inline budget: all of it when it fits once its
/// table structure is gone, else its head and its tail with the count left out between them
/// and how to see it.
#[must_use]
pub fn fit_interactive(full: &str, read_with: &str) -> String {
    let filtered = interactive_lines(full);
    if filtered.chars().count() <= INTERACTIVE_INLINE_MAX_CHARS {
        return filtered;
    }
    let mut lines = filtered.split('\n').collect::<Vec<_>>();
    let (open, close) = match (lines.first(), lines.last()) {
        (Some(first), Some(last))
            if first.starts_with(PAGE_CONTENT_OPEN) && last.starts_with("[END_UNTRUSTED") =>
        {
            let fence = (first.to_string(), last.to_string());
            lines.remove(0);
            lines.pop();
            fence
        }
        _ => (String::new(), String::new()),
    };
    let mut head = Vec::new();
    let mut used = 0;
    for line in &lines {
        if used + line.len() > INTERACTIVE_HEAD_CHARS {
            break;
        }
        used += line.len() + 1;
        head.push(*line);
    }
    let mut tail = Vec::new();
    let mut used = 0;
    for line in lines[head.len()..].iter().rev() {
        if used + line.len() > INTERACTIVE_INLINE_MAX_CHARS - INTERACTIVE_HEAD_CHARS {
            break;
        }
        used += line.len() + 1;
        tail.push(*line);
    }
    tail.reverse();
    let left_out = lines.len() - head.len() - tail.len();
    let fenced = |part: &[&str]| {
        if open.is_empty() {
            part.join("\n")
        } else {
            format!("{open}\n{}\n{close}", part.join("\n"))
        }
    };
    format!(
        "{}\n[{left_out} lines of the middle left out. Read them with: {read_with}, or search with: browseros grep <pattern>]\n{}",
        fenced(&head),
        fenced(&tail)
    )
}

/// A markdown read with each run of spaces cut to one, outside code blocks. The page's own
/// layout whitespace survives the conversion, and a cart page read back with a run of over a
/// thousand spaces between items. A list item keeps its indent, which is its nesting.
#[must_use]
pub fn collapse_spaces(text: &str) -> String {
    let mut in_code = false;
    map_lines(text, |line, in_page| {
        if !in_page {
            return Some(line.to_string());
        }
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            return Some(line.trim_end().to_string());
        }
        if in_code {
            return Some(line.to_string());
        }
        let body = line.trim_start();
        let indent = if is_list_item(body) {
            &line[..line.len() - body.len()]
        } else {
            ""
        };
        let mut out = String::with_capacity(line.len());
        out.push_str(indent);
        let mut previous_space = false;
        for ch in body.trim_end().chars() {
            let space = ch == ' ' || ch == '\t';
            if !(space && previous_space) {
                out.push(if space { ' ' } else { ch });
            }
            previous_space = space;
        }
        Some(out)
    })
}

fn is_list_item(body: &str) -> bool {
    body.starts_with("- ")
        || body.starts_with("* ")
        || body.starts_with("+ ")
        || body.split_once(". ").is_some_and(|(number, _)| {
            !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn saved() -> (String, Option<String>) {
        ("out-1".to_string(), Some("snapshot.md".to_string()))
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
    fn an_action_summary_names_the_command_to_see_the_page() {
        assert_eq!(
            cli_hints(
                "changed: 120 added, 0 removed. Take a snapshot to see details.",
                None
            ),
            "changed: 120 added, 0 removed. Run `browseros snapshot -i` to see the page now."
        );
        assert_eq!(
            cli_hints(
                "URL changed (https://a.test/ -> https://a.test/s?q=x); 9 added, 3 removed. Take a snapshot for the current state.",
                None
            ),
            "URL changed (https://a.test/ -> https://a.test/s?q=x); 9 added, 3 removed. Run `browseros snapshot -i` for the current state."
        );
    }

    #[test]
    fn the_saved_output_line_names_the_part_to_read() {
        let text = "ok\n[Saved output: out-1. Use read_output with outputId and part; omit part to list parts.]";
        assert_eq!(
            cli_hints(text, Some(&saved())),
            "ok\n[Saved output: out-1. Read it with: browseros output out-1 --part snapshot.md]"
        );
    }

    #[test]
    fn page_content_that_mentions_read_output_is_left_alone() {
        let text = "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] notice\nUse read_output for the full diff.\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\nUse read_output for the full diff.";
        assert_eq!(
            cli_hints(text, Some(&saved())),
            "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] notice\nUse read_output for the full diff.\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\nRead the full diff with: browseros output out-1 --part snapshot.md"
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

    const DATE_PICKER: &str =
        "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=https://www.google.com/travel/flights] notice
- dialog \"Departure\" [ref=e70]
  - grid \"November 2026\"
    - row \"Sunday, November 29, 2026\"
      - gridcell \"Monday, November 30, 2026, 10203 Indian rupees\" [ref=e81]
        - button \"Monday, November 30, 2026, 10203 Indian rupees\" [ref=e82]
    - row
      - gridcell \"Tuesday, December 1, 2026\"
        - button \"Tuesday, December 1, 2026, 11050 Indian rupees\" [ref=e84]
      - gridcell \"A cell with two\" [ref=e85]
        - button \"One\" [ref=e86]
        - button \"Two\" [ref=e87]
  - button \"Done. Search for one-way flights, departing on November 30, 2026\" [ref=e742]
[END_UNTRUSTED_PAGE_CONTENT nonce=ab]";

    #[test]
    fn a_date_picker_keeps_its_buttons_and_loses_the_grid_around_them() {
        assert_eq!(
            interactive_lines(DATE_PICKER),
            "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=https://www.google.com/travel/flights] notice
- dialog \"Departure\" [ref=e70]
  - grid \"November 2026\"
        - button \"Monday, November 30, 2026, 10203 Indian rupees\" [ref=e82]
        - button \"Tuesday, December 1, 2026, 11050 Indian rupees\" [ref=e84]
      - gridcell \"A cell with two\" [ref=e85]
        - button \"One\" [ref=e86]
        - button \"Two\" [ref=e87]
  - button \"Done. Search for one-way flights, departing on November 30, 2026\" [ref=e742]
[END_UNTRUSTED_PAGE_CONTENT nonce=ab]"
        );
    }

    #[test]
    fn a_row_outside_page_content_and_a_name_mentioning_a_ref_are_left_alone() {
        let text = "- row \"not page content\"\n[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] n\n- row \"names [ref=e1] in text\"\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]";
        assert_eq!(
            interactive_lines(text),
            "- row \"not page content\"\n[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] n\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]"
        );
    }

    #[test]
    fn a_long_snapshot_keeps_the_dialog_buttons_at_its_end() {
        let days = (1..=400)
            .map(|day| format!("    - row \"Week {day}\"\n      - gridcell \"Day {day}\"\n        - button \"Day {day}, 10203 Indian rupees, a price for this date\" [ref=e{day}]"))
            .collect::<Vec<_>>()
            .join("\n");
        let full = format!(
            "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=x] n\n- dialog \"Departure\" [ref=e1000]\n{days}\n  - button \"Done\" [ref=e742]\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]"
        );
        let fitted = fit_interactive(&full, "browseros output abc --part snapshot.md");
        assert!(fitted.chars().count() < INTERACTIVE_INLINE_MAX_CHARS + 1_000);
        assert!(fitted.contains("- dialog \"Departure\" [ref=e1000]"));
        assert!(fitted.contains("- button \"Done\" [ref=e742]"));
        assert!(fitted.contains(
            "lines of the middle left out. Read them with: browseros output abc --part snapshot.md"
        ));
        assert!(!fitted.contains("gridcell"));
        assert_eq!(
            fitted
                .matches("[END_UNTRUSTED_PAGE_CONTENT nonce=ab]")
                .count(),
            2
        );
    }

    #[test]
    fn a_snapshot_that_fits_once_filtered_is_printed_whole() {
        assert_eq!(
            fit_interactive(DATE_PICKER, "browseros output abc"),
            interactive_lines(DATE_PICKER)
        );
    }

    #[test]
    fn a_markdown_read_loses_its_runs_of_spaces_but_not_its_code_or_list_nesting() {
        let gap = " ".repeat(1_089);
        let text = format!(
            "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=https://www.amazon.in/cart] n\n# Shopping Cart\n{gap}Sensodyne Fresh Mint{gap}₹ 160.00   \n- Qty:  1\n  - Delete    Save for later\n```\nkeep    this\n```\n{gap}\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\noutside    the fence"
        );
        assert_eq!(
            collapse_spaces(&text),
            "[UNTRUSTED_PAGE_CONTENT nonce=ab origin=https://www.amazon.in/cart] n\n# Shopping Cart\nSensodyne Fresh Mint ₹ 160.00\n- Qty: 1\n  - Delete Save for later\n```\nkeep    this\n```\n\n[END_UNTRUSTED_PAGE_CONTENT nonce=ab]\noutside    the fence"
        );
    }
}
