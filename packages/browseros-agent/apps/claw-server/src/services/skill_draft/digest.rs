//! Builds the redacted digest of one finished task that the decisions worker turns into a skill
//! draft. It reads only what the task history already holds (the agent's summary, the tools it
//! called, the pages it was on, what neo did) and never page text, snapshots or screenshots. Every
//! string goes through [`super::redact`], and the whole digest stays under the worker's size cap.

use serde::Serialize;
use serde_json::Value;

use super::redact::{bare_host, cut, host_and_path, scrub_text, search_term};
use crate::db::{
    audit_log::{TaskDetail, TaskStatus, TaskSummary, ToolDispatchRow},
    pursue_runs::{PursueOutcome, PursueStepKind, pursue_run},
};

/// The worker refuses bodies over 32 KiB; staying well under leaves room for its own framing.
const MAX_DIGEST_BYTES: usize = 24 * 1024;
const MAX_STEPS: usize = 40;
const MAX_STEP_CHARS: usize = 200;
const MAX_PAGES: usize = 20;
const MAX_FIELDS: usize = 12;
const MAX_NEO_STEPS: usize = 20;
const MAX_GOAL_CHARS: usize = 300;
const MAX_TITLE_CHARS: usize = 120;
const MAX_PAGE_LABEL: usize = 60;

/// The body of `POST /v1/skills/draft`, in the worker's own field names.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRequest {
    /// One per draft attempt, reused on a retry so the worker counts it once.
    pub request_id: String,
    pub goal: String,
    pub outcome: &'static str,
    pub site: String,
    pub agent: &'static str,
    pub steps: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neo: Option<NeoPart>,
    pub pages: Vec<Page>,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeoPart {
    pub goal: String,
    pub handed_back: bool,
    pub done_steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Page {
    pub host: String,
    pub path: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Field {
    pub label: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl DraftRequest {
    /// Every host the digest names: the drafted skill may only point at these.
    #[must_use]
    pub fn hosts(&self) -> Vec<String> {
        let mut hosts = vec![self.site.clone()];
        for page in &self.pages {
            if !hosts.contains(&page.host) {
                hosts.push(page.host.clone());
            }
        }
        hosts
    }
}

/// Whether a task ended in a way a skill can be drafted from: finished, or stopped before any
/// call failed. A failed task, or one stopped after errors, would teach the skill its mistakes.
#[must_use]
pub fn ended_cleanly(summary: &TaskSummary) -> bool {
    match summary.status {
        TaskStatus::Done => true,
        TaskStatus::Cancelled => summary.error_count == 0,
        TaskStatus::Live | TaskStatus::Failed => false,
    }
}

/// The digest for a finished task, or `None` when it never reached a web page the worker would
/// accept as the skill's site or cannot be cut to fit the worker's size cap. `request_id` is left
/// empty for the caller to fill.
#[must_use]
pub fn build(task: &TaskDetail) -> Option<DraftRequest> {
    let summary = &task.summary;
    let rows: Vec<&ToolDispatchRow> = task
        .dispatches
        .iter()
        .filter(|row| row.parent_dispatch_id.is_none())
        .collect();
    // Pages and searches also come from calls a script made, so a task done through scripts
    // still shows where it went; only their URLs and titles are read, never the script or input.
    let every_row: Vec<&ToolDispatchRow> = task.dispatches.iter().collect();
    let site = summary
        .site
        .as_deref()
        .and_then(|site| bare_host(site.split('/').next().unwrap_or(site)))
        .or_else(|| {
            rows.iter()
                .find_map(|row| row.url.as_deref().and_then(host_and_path))
                .map(|(host, _)| host)
        })?;
    let goal_source = summary
        .task_summary
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(&summary.title);
    let mut goal = scrub_text(goal_source, MAX_GOAL_CHARS);
    if goal.is_empty() {
        goal = format!("Browse {site}");
    }
    let mut request = DraftRequest {
        request_id: String::new(),
        goal,
        outcome: "done",
        agent: agent_kind(&summary.slug),
        steps: steps(&rows),
        neo: neo_part(&task.dispatches),
        pages: pages(&every_row),
        fields: fields(&every_row),
        site,
    };
    fit(&mut request).then_some(request)
}

/// The agent family the worker knows, from the connected agent's id.
fn agent_kind(slug: &str) -> &'static str {
    let slug = slug.to_ascii_lowercase();
    if slug.starts_with("claude-code") {
        "claude-code"
    } else if slug.starts_with("claude") {
        "claude"
    } else if slug.starts_with("codex") {
        "codex"
    } else if slug.starts_with("cursor") {
        "cursor"
    } else {
        "other"
    }
}

/// Calls that only set up or check the agent's own state, never a step a person would take.
const BEHIND_THE_SCENES: [&str; 20] = [
    "browser_profiles",
    "browser_target",
    "name_session",
    "wait",
    "screenshot",
    "helpers",
    "helpers.save",
    "helpers.list",
    "helpers.read",
    "pages.list",
    "pages.getInfo",
    "pages.close",
    "observe.resolveRef",
    "cdp",
    "cdpJsonForPage",
    "mark_skill_run",
    "save_skill",
    "run_skill",
    "report_task",
    "ask_human",
];

/// What happened, one plain past-tense sentence per call, the same way the task page tells it.
fn steps(rows: &[&ToolDispatchRow]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for row in rows {
        let Some(sentence) = sentence(row) else {
            continue;
        };
        let sentence = scrub_text(&sentence, MAX_STEP_CHARS);
        if sentence.is_empty() || out.last() == Some(&sentence) {
            continue;
        }
        out.push(sentence);
    }
    trim_middle(&mut out, MAX_STEPS);
    out
}

fn sentence(row: &ToolDispatchRow) -> Option<String> {
    let tool = row.tool_name.as_str();
    if BEHIND_THE_SCENES.contains(&tool) || failed(row) {
        return None;
    }
    let args = args(row);
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let page = page_label(row).unwrap_or_else(|| "the page".to_string());
    Some(match tool {
        "navigate" | "nav.goto" | "pages.newPage" => format!("Opened {page}"),
        "tabs" | "windows" | "tab_groups" => match text("action").unwrap_or("list") {
            "new" | "open" | "create" => format!("Opened {page}"),
            _ => return None,
        },
        "act" => match text("kind").unwrap_or("click") {
            "type" | "type_at" | "fill" => format!("Typed into {page}"),
            "press" => format!("Pressed a key on {page}"),
            "scroll" => format!("Scrolled {page}"),
            "select" => format!("Picked an option on {page}"),
            "check" => format!("Ticked a box on {page}"),
            "uncheck" => format!("Unticked a box on {page}"),
            "hover" | "hover_at" | "focus" => format!("Moved to an item on {page}"),
            _ => format!("Clicked on {page}"),
        },
        "input.click" => format!("Clicked on {page}"),
        "input.fill" | "input.type" | "input.insertText" => format!("Typed into {page}"),
        "input.selectOption" => format!("Picked an option on {page}"),
        "input.scroll" => format!("Scrolled {page}"),
        "snapshot" | "read" | "grep" | "diff" | "observe.snapshot" | "observe.diff" | "pdf" => {
            format!("Read {page}")
        }
        "pursue" => match text("goal") {
            Some(goal) => format!("neo took over the clicking: \"{goal}\""),
            None => "neo took over the clicking".to_string(),
        },
        "run" | "evaluate" => format!("Ran a script on {page}"),
        "download" => format!("Downloaded a file from {page}"),
        "upload" => format!("Attached a file on {page}"),
        _ => return None,
    })
}

fn page_label(row: &ToolDispatchRow) -> Option<String> {
    row.title
        .as_deref()
        .map(|title| scrub_text(title, MAX_PAGE_LABEL))
        .filter(|title| !title.is_empty())
        .or_else(|| {
            row.url
                .as_deref()
                .and_then(host_and_path)
                .map(|(host, _)| host)
        })
}

/// What neo did when it took over: its goal, whether it handed the page back, and its steps
/// (only the option it chose or the site it opened, never text it typed).
fn neo_part(dispatches: &[ToolDispatchRow]) -> Option<NeoPart> {
    let run = pursue_run(dispatches)?;
    let goal = dispatches
        .iter()
        .rev()
        .filter(|row| row.tool_name == "pursue")
        .find_map(|row| {
            args(row)
                .get("goal")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|goal| scrub_text(&goal, MAX_GOAL_CHARS))
        .unwrap_or_default();
    let mut done_steps: Vec<String> = run
        .steps
        .iter()
        .map(|step| {
            let shown = matches!(step.kind, PursueStepKind::Select | PursueStepKind::Open)
                .then_some(step.text.as_deref())
                .flatten()
                .map(|text| scrub_text(text, 80))
                .filter(|text| !text.is_empty());
            let verb = match step.kind {
                PursueStepKind::Click => "Clicked",
                PursueStepKind::Type => "Typed",
                PursueStepKind::Press => "Pressed a key",
                PursueStepKind::Check => "Ticked a box",
                PursueStepKind::Uncheck => "Unticked a box",
                PursueStepKind::Select => "Picked",
                PursueStepKind::Scroll => "Scrolled",
                PursueStepKind::Drag => "Dragged",
                PursueStepKind::Dialog => "Answered a pop-up",
                PursueStepKind::Open => "Opened",
                PursueStepKind::Back => "Went back",
                PursueStepKind::Reload => "Reloaded",
            };
            match shown {
                Some(text) => cut(&format!("{verb} {text}"), MAX_STEP_CHARS),
                None => verb.to_string(),
            }
        })
        .collect();
    trim_middle(&mut done_steps, MAX_NEO_STEPS);
    Some(NeoPart {
        goal,
        handed_back: run.outcome == PursueOutcome::HandedBack,
        done_steps,
    })
}

/// The distinct pages the task was on, in order, as host plus a cleaned path.
fn pages(rows: &[&ToolDispatchRow]) -> Vec<Page> {
    let mut out: Vec<Page> = Vec::new();
    for row in rows {
        if BEHIND_THE_SCENES.contains(&row.tool_name.as_str()) {
            continue;
        }
        let Some((host, path)) = row.url.as_deref().and_then(host_and_path) else {
            continue;
        };
        if out
            .iter()
            .any(|page| page.host == host && page.path == path)
        {
            continue;
        }
        let title = row
            .title
            .as_deref()
            .map(|title| scrub_text(title, MAX_TITLE_CHARS))
            .unwrap_or_default();
        out.push(Page { host, path, title });
        if out.len() == MAX_PAGES {
            break;
        }
    }
    out
}

/// The searches the task ran, read from the search terms sites put in their own URLs.
fn fields(rows: &[&ToolDispatchRow]) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    for row in rows {
        if BEHIND_THE_SCENES.contains(&row.tool_name.as_str()) {
            continue;
        }
        let Some(term) = row.url.as_deref().and_then(search_term) else {
            continue;
        };
        if out
            .iter()
            .any(|field| field.value.as_deref() == Some(term.as_str()))
        {
            continue;
        }
        out.push(Field {
            label: "Search".to_string(),
            kind: "search",
            value: Some(term),
        });
        if out.len() == MAX_FIELDS {
            break;
        }
    }
    out
}

/// Cuts the digest until it fits, in order of what the draft can best do without: steps from the
/// middle (keeping how the task began and ended), then neo's steps, then later pages, then later
/// searches. False when even the smallest digest is over the cap.
fn fit(request: &mut DraftRequest) -> bool {
    let size =
        |request: &DraftRequest| serde_json::to_vec(request).map_or(usize::MAX, |body| body.len());
    while size(request) > MAX_DIGEST_BYTES {
        if request.steps.len() > 2 {
            let middle = request.steps.len() / 2;
            request.steps.remove(middle);
        } else if let Some(neo) = request
            .neo
            .as_mut()
            .filter(|neo| !neo.done_steps.is_empty())
        {
            let middle = neo.done_steps.len() / 2;
            neo.done_steps.remove(middle);
        } else if request.pages.len() > 1 {
            request.pages.pop();
        } else if !request.fields.is_empty() {
            request.fields.pop();
        } else {
            return false;
        }
    }
    true
}

fn trim_middle(items: &mut Vec<String>, max: usize) {
    while items.len() > max {
        let middle = items.len() / 2;
        items.remove(middle);
    }
}

fn args(row: &ToolDispatchRow) -> serde_json::Map<String, Value> {
    row.args_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn failed(row: &ToolDispatchRow) -> bool {
    row.result_meta
        .as_deref()
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
        .is_some_and(|meta| meta.get("isError").and_then(Value::as_bool) == Some(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: i64,
        tool: &str,
        url: &str,
        title: &str,
        args: serde_json::Value,
    ) -> ToolDispatchRow {
        ToolDispatchRow {
            id,
            created_at: 1_000 + id,
            agent_id: "agent".into(),
            slug: "claude-code".into(),
            agent_label: "Claude Code".into(),
            session_id: "sess".into(),
            tool_name: tool.into(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: Some(url.into()),
            title: Some(title.into()),
            args_json: Some(args.to_string()),
            result_meta: None,
            duration_ms: Some(10),
            tool_input_token_estimate: 0,
            tool_output_token_estimate: 0,
            token_estimator_version: 1,
            dispatch_id: None,
            parent_dispatch_id: None,
            has_screenshot: false,
        }
    }

    fn task(dispatches: Vec<ToolDispatchRow>) -> TaskDetail {
        TaskDetail {
            summary: TaskSummary {
                session_id: "sess".into(),
                agent_id: "agent".into(),
                slug: "claude-code".into(),
                agent_label: "Claude Code".into(),
                title: "Browsed www.flipkart.com".into(),
                site: Some("www.flipkart.com".into()),
                started_at: 1_000,
                ended_at: Some(2_000),
                duration_ms: 1_000,
                dispatch_count: dispatches.len() as i64,
                tool_sequence: vec![],
                status: TaskStatus::Done,
                error_count: 0,
                last_screenshot_dispatch_id: None,
                cursor_id: 1,
                has_screenshots: false,
                tool_input_token_estimate: 0,
                tool_output_token_estimate: 0,
                tokens_measured: false,
                task_summary: Some(
                    "Found phones under 30000 for jane.doe@example.com, call 9876543210".into(),
                ),
            },
            dispatches,
            screenshot_dispatch_ids: vec![],
            start_event: None,
            end_event: None,
        }
    }

    #[test]
    fn a_task_stopped_without_errors_can_become_a_skill() {
        let mut summary = task(vec![]).summary;
        assert!(ended_cleanly(&summary));
        summary.status = TaskStatus::Cancelled;
        assert!(ended_cleanly(&summary));
        summary.error_count = 1;
        assert!(!ended_cleanly(&summary));
        summary.error_count = 0;
        for status in [TaskStatus::Failed, TaskStatus::Live] {
            summary.status = status;
            assert!(!ended_cleanly(&summary));
        }
    }

    #[test]
    fn the_digest_keeps_the_task_and_drops_personal_values() -> anyhow::Result<()> {
        let detail = task(vec![
            row(
                1,
                "browser_target",
                "https://www.flipkart.com/",
                "Flipkart",
                serde_json::json!({}),
            ),
            row(
                2,
                "navigate",
                "https://www.flipkart.com/search?q=phones&otracker=abc123def456ghi789jkl",
                "Mobiles | Order 4111 1111 1111 1111",
                serde_json::json!({ "url": "https://www.flipkart.com/search?q=phones" }),
            ),
            row(
                3,
                "act",
                "https://www.flipkart.com/account/login?email=jane%40example.com",
                "Sign in",
                serde_json::json!({ "kind": "fill", "text": "hunter2" }),
            ),
            row(
                4,
                "act",
                "https://www.flipkart.com/apple-iphone/p/itm1234567",
                "iPhone",
                serde_json::json!({ "kind": "click" }),
            ),
        ]);
        let digest = build(&detail).ok_or_else(|| anyhow::anyhow!("no digest"))?;
        let body = serde_json::to_string(&digest)?;

        assert_eq!(digest.site, "flipkart.com");
        assert_eq!(digest.agent, "claude-code");
        assert!(digest.goal.starts_with("Found phones under 30000"));
        for leaked in [
            "jane",
            "example.com",
            "9876543210",
            "4111",
            "hunter2",
            "otracker",
            "itm1234567",
        ] {
            assert!(!body.contains(leaked), "{leaked} leaked: {body}");
        }
        assert_eq!(
            digest.steps,
            vec![
                "Opened Mobiles | Order [removed]",
                "Typed into Sign in",
                "Clicked on iPhone",
            ]
        );
        let paths: Vec<&str> = digest.pages.iter().map(|page| page.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["/search", "/account/login", "/apple-iphone/p/{id}"]
        );
        assert_eq!(digest.fields.len(), 1);
        assert_eq!(digest.fields[0].value.as_deref(), Some("phones"));
        assert_eq!(digest.hosts(), vec!["flipkart.com".to_string()]);
        assert!(digest.neo.is_none());
        Ok(())
    }

    #[test]
    fn a_task_that_never_reached_a_site_has_no_digest() {
        let mut detail = task(vec![]);
        detail.summary.site = None;
        assert!(build(&detail).is_none());
    }

    #[test]
    fn a_long_task_is_cut_from_the_middle() -> anyhow::Result<()> {
        let rows = (0..200)
            .map(|index| {
                row(
                    index,
                    "navigate",
                    &format!("https://www.flipkart.com/page{}", index % 2),
                    &format!("Page {index} {}", "x".repeat(50)),
                    serde_json::json!({}),
                )
            })
            .collect();
        let digest = build(&task(rows)).ok_or_else(|| anyhow::anyhow!("no digest"))?;
        assert_eq!(digest.steps.len(), MAX_STEPS);
        assert!(digest.steps[0].starts_with("Opened Page 0"));
        assert!(digest.steps[MAX_STEPS - 1].starts_with("Opened Page 199"));
        assert!(serde_json::to_vec(&digest)?.len() <= MAX_DIGEST_BYTES);
        Ok(())
    }

    #[test]
    fn long_non_english_pages_and_searches_still_fit() -> anyhow::Result<()> {
        let title = "स्मार्टफोन ".repeat(40);
        let rows = (0..60)
            .map(|index| {
                row(
                    index,
                    "navigate",
                    &format!(
                        "https://www.flipkart.com/{}{index}/x?q={}{index}",
                        "श्रेणी".repeat(10),
                        "फ़ोन".repeat(50)
                    ),
                    &title,
                    serde_json::json!({}),
                )
            })
            .collect();
        let digest = build(&task(rows)).ok_or_else(|| anyhow::anyhow!("no digest"))?;
        assert!(serde_json::to_vec(&digest)?.len() <= MAX_DIGEST_BYTES);
        assert!(!digest.pages.is_empty());
        assert_eq!(digest.site, "flipkart.com");
        Ok(())
    }

    #[test]
    fn pages_and_searches_inside_a_script_are_kept_without_its_source() -> anyhow::Result<()> {
        let mut script = row(
            1,
            "run",
            "https://www.flipkart.com/",
            "Flipkart",
            serde_json::json!({ "code": "await page.fill('#q', 'hunter2')" }),
        );
        script.dispatch_id = Some("script-1".into());
        let mut inner = row(
            2,
            "nav.goto",
            "https://www.flipkart.com/search?q=phones+under+30000",
            "Mobiles",
            serde_json::json!({ "text": "hunter2" }),
        );
        inner.parent_dispatch_id = Some("script-1".into());
        let digest =
            build(&task(vec![script, inner])).ok_or_else(|| anyhow::anyhow!("no digest"))?;
        assert!(
            digest.pages.iter().any(|page| page.path == "/search"),
            "{digest:?}"
        );
        assert_eq!(
            digest
                .fields
                .first()
                .and_then(|field| field.value.as_deref()),
            Some("phones under 30000")
        );
        assert_eq!(digest.steps, vec!["Ran a script on Flipkart".to_string()]);
        assert!(!serde_json::to_string(&digest)?.contains("hunter2"));
        Ok(())
    }
}
