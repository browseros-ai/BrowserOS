//! Command line in, tool call out, as a pure function.
//!
//! Nothing here reads state, the network or the disk. The executor resolves
//! external text first (stdin, a file, an environment variable) and hands it
//! in, so the whole mapping stays testable without a browser or a server.
//! That is where a surface of two dozen tools and forty-odd verbs actually
//! breaks, so it is where the main test lives.

use serde_json::{Map, Value, json};

use crate::cli::{
    Command, DialogCmd, DiffFlag, GroupCmd, HelpCmd, HelpersCmd, ProfileCmd, SessionCmd, SkillCmd,
    SkillsCmd, TabCmd, TargetCmd, WindowCmd,
};
use crate::exit::Failure;
use crate::refs;

/// A value the executor must read before the plan can be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum External {
    Stdin,
    File(String),
    Env(String),
}

/// One tool call, fully formed except for the fields only state can supply.
#[derive(Debug, Clone, Default)]
pub struct ToolPlan {
    pub tool: &'static str,
    pub args: Map<String, Value>,
    /// Inject the current page as `page`.
    pub needs_page: bool,
    /// Inject the remembered tab group as `groupId`, so work stays grouped.
    pub with_group: bool,
    /// Read the new page id out of the result and remember it.
    pub remember_page: bool,
    /// Where an inline image should be written. A terminal cannot render one.
    pub save_image: Option<Option<String>>,
    /// Bind `page` in a `run` script, so it loses the preamble that exists
    /// only because MCP has no current page.
    pub wrap_page: bool,
    /// Lines for stderr, decided here so they can be asserted in a test.
    pub notices: Vec<String>,
}

impl ToolPlan {
    fn new(tool: &'static str) -> Self {
        Self {
            tool,
            ..Self::default()
        }
    }

    fn arg(mut self, key: &str, value: Value) -> Self {
        if !value.is_null() {
            self.args.insert(key.to_string(), value);
        }
        self
    }

    fn page(mut self) -> Self {
        self.needs_page = true;
        self
    }
}

/// Something the CLI answers itself, with or without the server's help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Local {
    Status,
    Use(u32),
    SessionStart {
        task: Option<String>,
        category: Option<String>,
        summary: Option<String>,
    },
    SessionList,
    SessionEnd {
        close_tabs: bool,
        all: bool,
    },
    SkillsList,
    SkillsGet {
        name: String,
        full: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Plan {
    Tool(Box<ToolPlan>),
    Local(Local),
}

impl Plan {
    fn tool(plan: ToolPlan) -> Self {
        Self::Tool(Box::new(plan))
    }

    /// The tool a plan will call, for the coverage test.
    #[must_use]
    pub fn tool_name(&self) -> Option<&'static str> {
        match self {
            Self::Tool(plan) => Some(plan.tool),
            Self::Local(_) => None,
        }
    }
}

/// What the executor has to read before [`build`] can run.
#[must_use]
pub fn needs_external(command: &Command) -> Option<External> {
    match command {
        Command::Run { file, code, .. } | Command::Eval { file, code, .. } => {
            if let Some(file) = file {
                return Some(External::File(file.clone()));
            }
            if code.as_deref() == Some("-") {
                return Some(External::Stdin);
            }
            None
        }
        Command::Fill {
            value_stdin,
            value_env,
            ..
        }
        | Command::Type {
            value_stdin,
            value_env,
            ..
        } => {
            if *value_stdin {
                return Some(External::Stdin);
            }
            value_env.clone().map(External::Env)
        }
        Command::Helpers(HelpersCmd::Save { file, .. }) => {
            file.clone().map(External::File).or(Some(External::Stdin))
        }
        _ => None,
    }
}

/// Builds the plan for one command.
///
/// # Errors
/// Returns a usage failure for a malformed ref, a missing required value, or a
/// value the server would reject anyway. Validating locally matters because
/// each of those otherwise costs a round trip to learn.
pub fn build(command: &Command, external: Option<&str>) -> Result<Plan, Failure> {
    Ok(match command {
        Command::Status | Command::Session(SessionCmd::Status) => Plan::Local(Local::Status),
        Command::Use { page } => Plan::Local(Local::Use(*page)),
        Command::Session(SessionCmd::Start {
            task,
            category,
            summary,
        }) => Plan::Local(Local::SessionStart {
            task: task.clone(),
            category: category.clone(),
            summary: summary.clone(),
        }),
        Command::Session(SessionCmd::List) => Plan::Local(Local::SessionList),
        Command::Session(SessionCmd::End { close_tabs, all }) => Plan::Local(Local::SessionEnd {
            close_tabs: *close_tabs,
            all: *all,
        }),
        Command::Session(SessionCmd::Name {
            name,
            category,
            summary,
        }) => {
            if let Some(summary) = summary
                && summary.chars().count() > 200
            {
                return Err(Failure::usage(
                    "session summary is capped at 200 characters by the server.",
                ));
            }
            Plan::tool(
                ToolPlan::new("name_session")
                    .arg("name", json!(name))
                    .arg("category", opt(category))
                    .arg("summary", opt(summary)),
            )
        }
        Command::Skills(SkillsCmd::List) => Plan::Local(Local::SkillsList),
        Command::Skills(SkillsCmd::Get { name, full }) => Plan::Local(Local::SkillsGet {
            name: name.clone(),
            full: *full,
        }),

        // `tabs action=new` navigates and returns once the page has stopped
        // changing, so `open` is one call rather than a fused pair.
        Command::Open { url, keep_page } => {
            let mut plan = ToolPlan::new("tabs")
                .arg("action", json!("new"))
                .arg("url", json!(url));
            plan.with_group = true;
            plan.remember_page = !keep_page;
            Plan::tool(plan)
        }
        Command::Goto { url } => Plan::tool(
            ToolPlan::new("navigate")
                .page()
                .arg("action", json!("url"))
                .arg("url", json!(url)),
        ),
        Command::Back => Plan::tool(
            ToolPlan::new("navigate")
                .page()
                .arg("action", json!("back")),
        ),
        Command::Forward => Plan::tool(
            ToolPlan::new("navigate")
                .page()
                .arg("action", json!("forward")),
        ),
        Command::Reload => Plan::tool(
            ToolPlan::new("navigate")
                .page()
                .arg("action", json!("reload")),
        ),
        Command::History { max } => {
            Plan::tool(ToolPlan::new("history").arg("maxResults", json!(max)))
        }

        Command::Tab(TabCmd::List) => {
            Plan::tool(ToolPlan::new("tabs").arg("action", json!("list")))
        }
        Command::Tab(TabCmd::Active) => {
            Plan::tool(ToolPlan::new("tabs").arg("action", json!("active")))
        }
        Command::Tab(TabCmd::New { url }) => {
            let mut plan = ToolPlan::new("tabs")
                .arg("action", json!("new"))
                .arg("url", opt(url));
            plan.with_group = true;
            plan.remember_page = true;
            Plan::tool(plan)
        }
        Command::Tab(TabCmd::Close { page }) => match page {
            Some(page) => Plan::tool(
                ToolPlan::new("tabs")
                    .arg("action", json!("close"))
                    .arg("page", json!(page)),
            ),
            None => Plan::tool(ToolPlan::new("tabs").page().arg("action", json!("close"))),
        },

        Command::Group(GroupCmd::List) => {
            Plan::tool(ToolPlan::new("tab_groups").arg("action", json!("list")))
        }
        Command::Group(GroupCmd::Create {
            pages,
            title,
            group,
        }) => Plan::tool(
            ToolPlan::new("tab_groups")
                .arg("action", json!("create"))
                .arg("pages", json!(pages))
                .arg("title", opt(title))
                .arg("groupId", opt(group)),
        ),
        Command::Group(GroupCmd::Update {
            group,
            title,
            color,
            collapsed,
        }) => Plan::tool(
            ToolPlan::new("tab_groups")
                .arg("action", json!("update"))
                .arg("groupId", json!(group))
                .arg("title", opt(title))
                .arg("color", opt(color))
                .arg("collapsed", collapsed.map_or(Value::Null, Value::from)),
        ),
        Command::Group(GroupCmd::Ungroup { pages }) => Plan::tool(
            ToolPlan::new("tab_groups")
                .arg("action", json!("ungroup"))
                .arg("pages", json!(pages)),
        ),
        Command::Group(GroupCmd::Close { group }) => Plan::tool(
            ToolPlan::new("tab_groups")
                .arg("action", json!("close"))
                .arg("groupId", json!(group)),
        ),

        Command::Window(WindowCmd::List) => {
            Plan::tool(ToolPlan::new("windows").arg("action", json!("list")))
        }
        Command::Window(WindowCmd::Create) => {
            Plan::tool(ToolPlan::new("windows").arg("action", json!("create")))
        }
        Command::Window(WindowCmd::Close { window }) => Plan::tool(
            ToolPlan::new("windows")
                .arg("action", json!("close"))
                .arg("windowId", json!(window)),
        ),

        Command::Profile(ProfileCmd::List) => {
            Plan::tool(ToolPlan::new("browser_profiles").arg("action", json!("list")))
        }
        Command::Profile(ProfileCmd::Load { name, profile_id }) => Plan::tool(
            ToolPlan::new("browser_profiles")
                .arg("action", json!("load"))
                .arg("name", opt(name))
                .arg("browserProfileId", opt(profile_id)),
        ),
        Command::Target(TargetCmd::Get) => {
            Plan::tool(ToolPlan::new("browser_target").arg("action", json!("get")))
        }
        Command::Target(TargetCmd::Set { name, profile_id }) => Plan::tool(
            ToolPlan::new("browser_target")
                .arg("action", json!("set"))
                .arg("name", opt(name))
                .arg("browserProfileId", opt(profile_id)),
        ),
        Command::Target(TargetCmd::SetWindow { window, strict }) => Plan::tool(
            ToolPlan::new("browser_target")
                .arg("action", json!("set_window"))
                .arg("windowId", json!(window))
                .arg("strict", json!(strict)),
        ),
        Command::Target(TargetCmd::ClearWindow) => {
            Plan::tool(ToolPlan::new("browser_target").arg("action", json!("clear_window")))
        }
        Command::Target(TargetCmd::Clear) => {
            Plan::tool(ToolPlan::new("browser_target").arg("action", json!("clear")))
        }

        Command::Snapshot { interactive, depth } => {
            if let Some(depth) = depth
                && (*depth < 1 || *depth > 100)
            {
                return Err(Failure::usage("snapshot --depth is clamped to 1..=100."));
            }
            Plan::tool(
                ToolPlan::new("snapshot")
                    .page()
                    .arg(
                        "mode",
                        json!(if *interactive { "interactive" } else { "full" }),
                    )
                    .arg("depth", depth.map_or(Value::Null, Value::from)),
            )
        }
        Command::Diff => Plan::tool(ToolPlan::new("diff").page()),
        Command::Read {
            selector,
            text,
            links,
            console,
            viewport,
            include_links,
            include_images,
        } => {
            let format = if *console {
                "console"
            } else if *links {
                "links"
            } else if *text {
                "text"
            } else {
                "markdown"
            };
            Plan::tool(
                ToolPlan::new("read")
                    .page()
                    .arg("format", json!(format))
                    .arg("selector", opt(selector))
                    .arg("viewportOnly", flag(*viewport))
                    .arg("includeLinks", flag(*include_links))
                    .arg("includeImages", flag(*include_images)),
            )
        }
        Command::Grep {
            pattern,
            over,
            limit,
        } => {
            if over != "ax" && over != "content" {
                return Err(Failure::usage("grep --over takes \"ax\" or \"content\"."));
            }
            Plan::tool(
                ToolPlan::new("grep")
                    .page()
                    .arg("pattern", json!(pattern))
                    .arg("over", json!(over))
                    .arg("limit", json!(limit)),
            )
        }
        Command::Screenshot {
            path,
            full,
            annotate,
            format,
            quality,
        } => {
            if !matches!(format.as_str(), "jpeg" | "png" | "webp") {
                return Err(Failure::usage(
                    "screenshot --format takes jpeg, png or webp.",
                ));
            }
            if let Some(quality) = quality
                && !(0..=100).contains(quality)
            {
                return Err(Failure::usage("screenshot --quality is 0..=100."));
            }
            let mut plan = ToolPlan::new("screenshot")
                .page()
                .arg("format", json!(format))
                .arg("fullPage", flag(*full))
                .arg("annotate", flag(*annotate))
                .arg("quality", quality.map_or(Value::Null, Value::from));
            plan.save_image = Some(path.clone());
            Plan::tool(plan)
        }
        Command::Pdf {
            path,
            landscape,
            no_background,
        } => {
            let mut plan = ToolPlan::new("pdf")
                .page()
                .arg("landscape", flag(*landscape))
                .arg("printBackground", json!(!no_background));
            if let Some(path) = path {
                plan.notices.push(format!(
                    "[pdf is written by the browser; {path} is where this CLI will copy it]"
                ));
            }
            Plan::tool(plan)
        }

        Command::Click {
            reference,
            at,
            button,
            count,
            diff,
        } => {
            let mut plan = act_base(if at.is_some() { "click_at" } else { "click" }, diff)?;
            plan = apply_target(plan, reference.as_deref(), at.as_deref(), "click")?;
            if !matches!(button.as_str(), "left" | "middle" | "right") {
                return Err(Failure::usage(
                    "click --button takes left, middle or right.",
                ));
            }
            Plan::tool(
                plan.arg("button", json!(button))
                    .arg("clickCount", count.map_or(Value::Null, Value::from)),
            )
        }
        Command::Fill {
            reference,
            value,
            fields,
            append,
            sensitive,
            ..
        } => {
            let plan = act_base("fill", &DiffFlag::default())?;
            let mut plan = plan;
            plan.args.remove("diff");
            let plan = reapply_diff(plan, command)?;
            if !fields.is_empty() {
                let parsed = fields
                    .iter()
                    .map(|raw| {
                        refs::field_pair(raw)
                            .map(|(reference, value)| json!({ "ref": reference, "value": value }))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(Plan::tool(
                    plan.arg("fields", Value::Array(parsed))
                        .arg("clear", json!(!append))
                        .arg("sensitive", flag(*sensitive)),
                ));
            }
            let Some(reference) = reference else {
                return Err(Failure::usage(
                    "fill needs a ref, or --fields @e1=\"a\" @e2=\"b\" to do a whole form in one call.",
                ));
            };
            let text = external
                .map(ToString::to_string)
                .or_else(|| value.clone())
                .ok_or_else(|| {
                    Failure::usage(
                        "fill needs a value. Use --value-stdin or --value-env for anything secret:\n  a CLI argument lands in shell history and in ps output.",
                    )
                })?;
            Plan::tool(
                plan.arg("ref", json!(refs::normalize(reference)?))
                    .arg("value", json!(text))
                    .arg("clear", json!(!append))
                    .arg("sensitive", flag(*sensitive)),
            )
        }
        Command::Type {
            text,
            sensitive,
            diff,
            ..
        } => {
            let plan = act_base("type", diff)?;
            let body = external
                .map(ToString::to_string)
                .or_else(|| text.clone())
                .ok_or_else(|| {
                    Failure::usage(
                        "type needs text. Use --value-stdin or --value-env for anything secret.",
                    )
                })?;
            Plan::tool(
                plan.arg("text", json!(body))
                    .arg("sensitive", flag(*sensitive)),
            )
        }
        Command::Press { key, diff } => Plan::tool(act_base("press", diff)?.arg("key", json!(key))),
        Command::Hover {
            reference,
            at,
            diff,
        } => {
            let plan = act_base(if at.is_some() { "hover_at" } else { "hover" }, diff)?;
            Plan::tool(apply_target(
                plan,
                reference.as_deref(),
                at.as_deref(),
                "hover",
            )?)
        }
        Command::Focus { reference, diff } => {
            Plan::tool(act_base("focus", diff)?.arg("ref", json!(refs::normalize(reference)?)))
        }
        Command::Check { reference, diff } => {
            Plan::tool(act_base("check", diff)?.arg("ref", json!(refs::normalize(reference)?)))
        }
        Command::Uncheck { reference, diff } => {
            Plan::tool(act_base("uncheck", diff)?.arg("ref", json!(refs::normalize(reference)?)))
        }
        Command::Select {
            reference,
            value,
            diff,
        } => Plan::tool(
            act_base("select", diff)?
                .arg("ref", json!(refs::normalize(reference)?))
                .arg("value", json!(value)),
        ),
        Command::Scroll {
            direction,
            amount,
            reference,
            diff,
        } => {
            if !matches!(direction.as_str(), "up" | "down" | "left" | "right") {
                return Err(Failure::usage("scroll takes up, down, left or right."));
            }
            let mut plan = act_base("scroll", diff)?
                .arg("direction", json!(direction))
                .arg("amount", amount.map_or(Value::Null, Value::from));
            if let Some(reference) = reference {
                plan = plan.arg("ref", json!(refs::normalize(reference)?));
            }
            Plan::tool(plan)
        }
        Command::Drag { from, to, diff } => Plan::tool(
            act_base("drag", diff)?
                .arg("ref", json!(refs::normalize(from)?))
                .arg("targetRef", json!(refs::normalize(to)?)),
        ),
        Command::Dialog(DialogCmd::Accept) => {
            Plan::tool(act_base("dialog_accept", &DiffFlag::default())?)
        }
        Command::Dialog(DialogCmd::Dismiss) => {
            Plan::tool(act_base("dialog_dismiss", &DiffFlag::default())?)
        }
        Command::Upload { reference, files } => Plan::tool(
            ToolPlan::new("upload")
                .page()
                .arg("ref", json!(refs::normalize(reference)?))
                .arg("files", json!(files)),
        ),
        Command::Download { reference } => Plan::tool(
            ToolPlan::new("download")
                .page()
                .arg("ref", json!(refs::normalize(reference)?)),
        ),

        Command::Wait {
            text,
            selector,
            time,
            timeout,
        } => {
            // Validated here because the server rejects an over-long pause
            // rather than shortening it, and learning that costs a round trip.
            if let Some(time) = time
                && *time > 90_000.0
            {
                return Err(Failure::usage(
                    "wait --time is honored up to 90000 ms, and a larger value is rejected rather than shortened.",
                ));
            }
            if let Some(timeout) = timeout
                && *timeout > 30_000.0
                && time.is_none()
            {
                return Err(Failure::usage(
                    "wait --timeout is capped at 30000 ms for --text and --selector.",
                ));
            }
            let (kind, value) = match (text, selector, time) {
                (Some(text), _, _) => ("text", Value::String(text.clone())),
                (_, Some(selector), _) => ("selector", Value::String(selector.clone())),
                (_, _, Some(time)) => ("time", Value::from(*time)),
                _ => {
                    return Err(Failure::usage(
                        "wait needs --text, --selector or --time. Waiting on the thing beats waiting on the clock.",
                    ));
                }
            };
            Plan::tool(
                ToolPlan::new("wait")
                    .page()
                    .arg("for", json!(kind))
                    .arg("value", value)
                    .arg("timeout", timeout.map_or(Value::Null, Value::from)),
            )
        }

        Command::Run {
            code,
            script_timeout,
            no_page,
            ..
        } => {
            let body = script_body(external, code.as_deref(), "run")?;
            let mut plan = ToolPlan::new("run").arg("code", json!(body));
            plan.wrap_page = !no_page;
            if let Some(requested) = script_timeout {
                let clamped = requested.min(30_000.0);
                if *requested > 30_000.0 {
                    plan.notices.push(format!(
                        "[run --script-timeout {requested} ms clamped to 30000: the server's cap is hard and a larger value is clamped, not honored]"
                    ));
                }
                plan = plan.arg("timeout", Value::from(clamped));
            }
            Plan::tool(plan)
        }
        Command::Eval {
            code,
            settle,
            max_chars,
            script_timeout,
            ..
        } => {
            if let Some(max_chars) = max_chars
                && *max_chars > 200_000
            {
                return Err(Failure::usage(
                    "eval --max-chars is capped at 200000, measured in UTF-8 bytes rather than characters.",
                ));
            }
            let body = script_body(external, code.as_deref(), "eval")?;
            let mut plan = ToolPlan::new("evaluate")
                .page()
                .arg("code", json!(body))
                .arg("settle", flag(*settle))
                .arg("maxChars", max_chars.map_or(Value::Null, Value::from));
            if let Some(requested) = script_timeout {
                let clamped = requested.min(30_000.0);
                if *requested > 30_000.0 {
                    plan.notices.push(format!(
                        "[eval --script-timeout {requested} ms clamped to 30000: the server's cap is hard]"
                    ));
                }
                plan = plan.arg("timeout", Value::from(clamped));
            }
            Plan::tool(plan)
        }
        // Helpers only work inside run, despite the roster being advertised in
        // other tools' output, so these are run scripts rather than tools.
        Command::Helpers(HelpersCmd::List) => Plan::tool(
            ToolPlan::new("run").arg("code", json!("return await browser.listHelpers({});")),
        ),
        Command::Helpers(HelpersCmd::Read { name }) => Plan::tool(ToolPlan::new("run").arg(
            "code",
            json!(format!(
                "return await browser.readHelper({}, {{}});",
                json!(name)
            )),
        )),
        Command::Helpers(HelpersCmd::Save { name, .. }) => {
            let source = external.ok_or_else(|| {
                Failure::usage("helpers save needs -f <file>, or the source on stdin.")
            })?;
            Plan::tool(ToolPlan::new("run").arg(
                "code",
                json!(format!(
                    "return await browser.saveHelper({}, {}, {{}});",
                    json!(name),
                    json!(source)
                )),
            ))
        }

        Command::Skill(SkillCmd::Save {
            name,
            description,
            steps,
            site,
            learned,
        }) => Plan::tool(
            ToolPlan::new("save_skill")
                .arg("name", json!(name))
                .arg("description", json!(description))
                .arg("steps", opt(steps))
                .arg("site", opt(site))
                .arg("learnedNotes", opt(learned)),
        ),
        Command::Skill(SkillCmd::Ran { name }) => {
            Plan::tool(ToolPlan::new("mark_skill_run").arg("name", json!(name)))
        }
        Command::Help(HelpCmd::Request {
            reason,
            details,
            kind,
            resume_hint,
        }) => Plan::tool(
            ToolPlan::new("request_human_help")
                .arg("reason", json!(reason))
                .arg("details", opt(details))
                .arg("kind", opt(kind))
                .arg("resumeHint", opt(resume_hint)),
        ),
        Command::Help(HelpCmd::Wait { request }) => {
            Plan::tool(ToolPlan::new("await_human_help").arg("requestId", opt(request)))
        }
        Command::Pursue {
            goal,
            value,
            destination,
        } => {
            let mut values = Map::new();
            for pair in value {
                let (key, text) = pair.split_once('=').ok_or_else(|| {
                    Failure::usage(format!("pursue --value takes name=value, got: {pair}"))
                })?;
                // The text is typed into the page and recorded with the call
                // like any other argument, which is why this is said out loud.
                values.insert(key.to_string(), Value::String(text.to_string()));
            }
            let mut destinations = Vec::new();
            for pair in destination {
                let (label, url) = pair.split_once('=').ok_or_else(|| {
                    Failure::usage(format!("pursue --destination takes label=url, got: {pair}"))
                })?;
                destinations.push(json!({ "label": label, "url": url }));
            }
            Plan::tool(
                ToolPlan::new("pursue")
                    .page()
                    .arg("goal", json!(goal))
                    .arg(
                        "values",
                        if values.is_empty() {
                            Value::Null
                        } else {
                            Value::Object(values)
                        },
                    )
                    .arg(
                        "destinations",
                        if destinations.is_empty() {
                            Value::Null
                        } else {
                            Value::Array(destinations)
                        },
                    ),
            )
        }
    })
}

fn act_base(kind: &'static str, diff: &DiffFlag) -> Result<ToolPlan, Failure> {
    let plan = ToolPlan::new("act").page().arg("kind", json!(kind));
    Ok(match &diff.diff {
        Some(raw) => plan.arg("diff", parse_diff(raw)?),
        None => plan,
    })
}

/// `fill` builds its plan before it knows its own diff flag, because the flag
/// lives on the variant. This puts it back.
fn reapply_diff(plan: ToolPlan, command: &Command) -> Result<ToolPlan, Failure> {
    let Command::Fill { diff, .. } = command else {
        return Ok(plan);
    };
    Ok(match &diff.diff {
        Some(raw) => plan.arg("diff", parse_diff(raw)?),
        None => plan,
    })
}

/// `act`'s `diff` is an untagged union of a mode and a character cap.
fn parse_diff(raw: &str) -> Result<Value, Failure> {
    match raw {
        "none" | "summary" | "full" => Ok(Value::String(raw.to_string())),
        other => other.parse::<f64>().map(Value::from).map_err(|_| {
            Failure::usage("--diff takes none, summary, full, or a number capping the inline diff.")
        }),
    }
}

fn apply_target(
    plan: ToolPlan,
    reference: Option<&str>,
    at: Option<&str>,
    verb: &str,
) -> Result<ToolPlan, Failure> {
    if let Some(at) = at {
        let (x, y) = at.split_once(',').ok_or_else(|| {
            Failure::usage(format!("{verb} --at takes X,Y viewport coordinates."))
        })?;
        let x: f64 = x
            .trim()
            .parse()
            .map_err(|_| Failure::usage(format!("{verb} --at takes numbers, got: {at}")))?;
        let y: f64 = y
            .trim()
            .parse()
            .map_err(|_| Failure::usage(format!("{verb} --at takes numbers, got: {at}")))?;
        return Ok(plan.arg("x", Value::from(x)).arg("y", Value::from(y)));
    }
    let reference = reference.ok_or_else(|| {
        Failure::usage(format!(
            "{verb} needs a ref from the last snapshot, or --at X,Y."
        ))
    })?;
    Ok(plan.arg("ref", json!(refs::normalize(reference)?)))
}

fn script_body(external: Option<&str>, code: Option<&str>, verb: &str) -> Result<String, Failure> {
    if let Some(external) = external {
        return Ok(external.to_string());
    }
    match code {
        Some("-") | None => Err(Failure::usage(format!(
            "{verb} needs a script. Pass it inline, with -f <file>, or as `-` to read stdin:\n  browseros {verb} - <<'JS'\n  …\n  JS"
        ))),
        Some(code) => Ok(code.to_string()),
    }
}

fn opt(value: &Option<String>) -> Value {
    value.clone().map_or(Value::Null, Value::String)
}

/// Only sends a boolean when it is true, so a default never rides along.
fn flag(value: bool) -> Value {
    if value {
        Value::Bool(true)
    } else {
        Value::Null
    }
}
