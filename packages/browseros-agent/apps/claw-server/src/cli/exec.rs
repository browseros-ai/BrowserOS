//! Running a plan: resolve what only state knows, call, then print and exit.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};

use crate::cli::client::{self, Client, result::ToolOutput};
use crate::cli::exit::{Code, Failure};
use crate::cli::grammar::{Cli, Command, Globals};
use crate::cli::plan::{self, External, Local, Plan, ToolPlan};
use crate::cli::{capability, guides, install, io, render, script, state};

/// Parses, plans and runs one invocation.
///
/// # Errors
/// Any failure already classified with an exit code.
pub async fn run(cli: Cli) -> Result<Code> {
    io::set_quiet(cli.globals.quiet);
    let external = match plan::needs_external(&cli.command) {
        Some(source) => Some(read_external(&source)?),
        None => None,
    };
    let built = plan::build(&cli.command, external.as_deref())?;
    let label = command_label(&cli.command);

    // Reading a guide or listing scope files must work with no server and no
    // browser: an agent learning the API should not have to start one first.
    if let Plan::Local(local) = &built
        && let Some(code) = run_offline(local)?
    {
        return Ok(code);
    }

    let timeout = cli.globals.client_timeout.map_or(
        client::transport::DEFAULT_READ_TIMEOUT,
        Duration::from_millis,
    );
    let agent = cli.globals.agent.clone().or_else(|| {
        std::env::var("BROWSEROS_AGENT")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let mut client =
        client::connect(cli.globals.endpoint.as_deref(), agent.as_deref(), timeout).await?;

    match built {
        Plan::Tool(tool) => run_tool(&mut client, &cli.globals, label, *tool).await,
        Plan::Local(local) => run_local(&mut client, &cli.globals, local).await,
    }
}

async fn run_tool(
    client: &mut Client,
    globals: &Globals,
    label: &'static str,
    mut plan: ToolPlan,
) -> Result<Code> {
    for notice in &plan.notices {
        io::note(notice);
    }

    // Before anything else, including resolving a page. A tool this server
    // does not have cannot be fixed by opening one, so "no current page" would
    // send the caller after the wrong problem.
    if let Some(reason) = capability::unavailable(plan.tool, &client.state().tools) {
        return Err(Failure::tool(format!(
            "this server does not offer `{}`: {reason}.\n  Run `browseros status` to see what it does offer.",
            plan.tool
        ))
        .into());
    }

    // A tool that is neither offered nor one of the known-optional ones means
    // the catalogue moved under us: a rename upstream, or a server too old or
    // too new for this binary. Saying which tool is missing turns an opaque
    // `tools/call` failure into something the caller can act on. Found when
    // `evaluate` became `evaluate_in_page` and `eval` failed with nothing but
    // the method name.
    if !client.state().tools.is_empty() && !client.state().offers(plan.tool) {
        return Err(Failure::tool(format!(
            "this server has no `{}` tool, so `browseros {}` cannot run.\n  The server is {}; this CLI may be older or newer than it.\n  Run `browseros status` to see what it offers.",
            plan.tool,
            label,
            client
                .state()
                .server_version
                .as_deref()
                .unwrap_or("an unknown version"),
        ))
        .into());
    }

    let page = if plan.needs_page {
        let page = globals
            .page
            .or(client.state().page)
            .ok_or_else(|| Failure::usage(
                "no current page in this directory.\n  Open one:  browseros open <url>\n  Or pick one: browseros tab list, then browseros use <id>",
            ))?;
        plan.args.insert("page".to_string(), json!(page));
        Some(page)
    } else {
        globals.page
    };

    // `run` has no page argument: the page is bound inside the script
    // instead, which is the whole preamble a CLI can write for the caller.
    if plan.wrap_page
        && let Some(page) = globals.page.or(client.state().page)
        && let Some(Value::String(code)) = plan.args.get("code")
    {
        let wrapped = script::with_page(code, page);
        plan.args.insert("code".to_string(), Value::String(wrapped));
    }

    if plan.with_group
        && let Some(group) = client.state().group_id.clone()
    {
        plan.args
            .entry("groupId".to_string())
            .or_insert(Value::String(group));
    }

    // The server rejects a timeout above its budget rather than clamping it,
    // so shorten it here and say so. The budget is the deployment's, read from
    // the run tool's schema; with none cached, the request goes as asked and
    // the server's own rejection is the authoritative answer.
    if let Some(requested) = plan.requested_timeout
        && let Some(budget) = client.state().run_budget_ms
        && requested > budget as f64
    {
        plan.args
            .insert("timeout".to_string(), Value::from(budget as f64));
        io::note(&format!(
            "[--script-timeout {requested} ms shortened to this server's {budget} ms budget, which it would otherwise reject]"
        ));
    }

    let started = Instant::now();
    let arguments = Value::Object(plan.args.clone());
    let had_task = client.state().group_id.is_some();
    let mut output = client.call(plan.tool, arguments).await?;

    // Naming a task that is not there, because none has started or its group was closed,
    // names the next one instead. The server's own reply would describe renaming a task
    // that is over, so it is not shown.
    if label == "task" && plan.tool == "name_session" && client.state().group_id.is_none() {
        let pending = state::PendingTask {
            name: plan
                .args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            category: plan
                .args
                .get("category")
                .and_then(Value::as_str)
                .map(str::to_string),
            summary: plan
                .args
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        let name = pending.name.clone();
        client.persist(|state| state.pending_task = Some(pending))?;
        io::out(&if had_task {
            format!(
                "this directory's task ended when its tab group was closed, so \"{name}\" will name the next task."
            )
        } else {
            format!("no task has started here yet, so \"{name}\" will name the next one.")
        });
        return Ok(Code::Ok);
    }

    // The task's group was closed without this CLI, by the user or by the
    // browser restarting, so placing a new tab in it is refused. The task went
    // with its group, so start a new one in a new group rather than leave the
    // directory wedged. Once, because a second refusal is about something else.
    if output.is_error
        && output.text.contains("group_out_of_scope")
        && plan.args.remove("groupId").is_some()
    {
        client.persist(|state| {
            state.group_id = None;
            state.task = None;
        })?;
        io::note(
            "[this directory's task ended when its tab group was closed, so this starts a new task in a new group]",
        );
        output = client
            .call(plan.tool, Value::Object(plan.args.clone()))
            .await?;
    }
    if plan.read_all && !output.is_error {
        let chunks = read_rest_of_output(client, &plan.args, &mut output).await?;
        if chunks > 1 {
            io::note(&format!("[read the whole part in {chunks} chunks]"));
        }
    } else if plan.tool == "read_output" {
        note_rest_of_output(&plan.args, &output);
    }
    let elapsed = started.elapsed().as_millis();

    if plan.remember_page
        && let Some(new_page) = render::page_from(&output)
    {
        let url = render::url_from(&output);
        client.persist(|state| {
            state.page = Some(new_page);
            state.page_url = url.clone();
        })?;
    }

    // `goto` and the other navigations keep the recorded url in step, so a
    // recycled page id can still be spotted.
    if !plan.remember_page
        && plan.needs_page
        && plan.tool == "navigate"
        && let Some(url) = render::url_from(&output)
    {
        client.persist(|state| state.page_url = Some(url.clone()))?;
    }

    if client.state().group_id.is_some()
        && let Some(pending) = client.state().pending_task.clone()
    {
        name_pending_task(client, pending).await?;
    }

    let mut files = Vec::new();
    if let Some(target) = plan.save_image.clone() {
        files = write_images(&output, target.as_deref())?;
    }
    if let Some(destination) = plan.copy_to.clone()
        && !output.is_error
        && let Some(written) = render::written_path(&output)
    {
        std::fs::copy(&written, &destination).map_err(|error| {
            Failure::tool(format!(
                "the browser wrote {written} but it could not be copied to {destination}: {error}"
            ))
        })?;
        files.push(destination);
    }

    if plan.wrap_page
        && page.is_none()
        && output.is_error
        && output.text.contains("page is not defined")
    {
        io::note(
            "[there is no current page, so `page` was not defined for this script.\n Open one with: browseros open <url>, or use browser.open(url) inside the script]",
        );
    }

    if script::hit_the_cap(&output) {
        // The logs are the body rather than a footnote: they survive the kill
        // and are the only evidence of how far the script got.
        if globals.json {
            io::out(&render::envelope(
                label, plan.tool, page, &output, elapsed, &files,
            ));
        } else if let Some(logs) = script::surviving_logs(&output) {
            io::out(&logs);
        }
        io::note(&script::cap_advice(client.state().run_budget_ms));
        return Ok(Code::Timeout);
    }

    // Last, after everything that reads the result: the page id and url are parsed out of the
    // full text, and only the printed copy is shortened.
    let navigates = plan.tool == "navigate"
        || (plan.tool == "tabs" && plan.args.get("action").and_then(Value::as_str) == Some("new"));
    if navigates {
        output.text =
            render::summarize_auto_snapshot(&output.text, render::AUTO_SNAPSHOT_MAX_CHARS);
    }
    report(&output, label, plan.tool, page, elapsed, &files, globals);

    // A page-scoped failure is usually a closed or recycled tab, and the one
    // extra round trip to say so precisely only happens on the failure path.
    if output.is_error && plan.needs_page {
        explain_stale_page(client, page).await;
    }
    Ok(output.exit_code())
}

/// The locals that need neither the network nor a browser.
fn run_offline(local: &Local) -> Result<Option<Code>> {
    match local {
        Local::GuideList => {
            io::out(&guides::list());
            Ok(Some(Code::Ok))
        }
        Local::GuideGet { name, full } => match guides::get(name, *full) {
            Some(body) => {
                io::out(body);
                Ok(Some(Code::Ok))
            }
            None => Err(Failure::usage(format!(
                "no guide named {name}. Available:\n{}",
                guides::list()
            ))
            .into()),
        },
        Local::TaskList => {
            task_list();
            Ok(Some(Code::Ok))
        }
        Local::InstallCli { confirm } => {
            let path = std::env::var("PATH").ok();
            install::run(*confirm, path.as_deref()).map(Some)
        }
        _ => Ok(None),
    }
}

async fn run_local(client: &mut Client, globals: &Globals, local: Local) -> Result<Code> {
    match local {
        Local::Status => status(client, globals).await,
        Local::Use(page) => {
            client.persist(|state| {
                state.page = Some(page);
                state.page_url = None;
            })?;
            io::out(&format!("page {page}"));
            Ok(Code::Ok)
        }
        Local::TaskStart {
            name,
            category,
            summary,
        } => task_start(client, name, category, summary),
        Local::TaskDone { keep_tabs, all } => task_done(client, keep_tabs, all).await,
        // Already answered by run_offline before the server was contacted.
        Local::GuideList | Local::GuideGet { .. } | Local::TaskList | Local::InstallCli { .. } => {
            Ok(Code::Ok)
        }
    }
}

async fn status(client: &mut Client, globals: &Globals) -> Result<Code> {
    let ready = client.transport().ready().await;
    // The scope file only remembers. One cheap call lets the server say whether the task is
    // still open, so a task whose tabs were closed is not reported as current.
    if client.state().group_id.is_some() && matches!(ready, Some((200, _))) {
        let _ = client.call("tabs", json!({ "action": "list" })).await;
    }
    let link = client::link_summary(ready.clone());
    // One tools/list, cached against the server version. It is the largest
    // response the CLI ever reads, so asking once per server version rather
    // than per command is the whole cost argument for a CLI over MCP.
    let offered = client.refresh_tools().await.unwrap_or_default();
    let state = client.state().clone();

    let rows = vec![
        (
            "endpoint",
            format!("{}  ({link})", client.transport().endpoint()),
        ),
        (
            "task",
            state
                .task
                .clone()
                .or_else(|| {
                    state
                        .pending_task
                        .as_ref()
                        .map(|pending| pending.name.clone())
                })
                .unwrap_or_default(),
        ),
        (
            "group",
            state
                .group_id
                .as_deref()
                .map(client::short)
                .unwrap_or_else(|| "none yet: the next page you open starts a task".to_string()),
        ),
        (
            "page",
            match (state.page, state.page_url.as_deref()) {
                (Some(page), Some(url)) => format!("{page}  {url}"),
                (Some(page), None) => page.to_string(),
                _ => String::new(),
            },
        ),
        ("scope", state.scope_path.clone().unwrap_or_default()),
        (
            "server",
            state
                .server_version
                .clone()
                .map(|version| format!("{version}  {} tools", offered.len()))
                .unwrap_or_default(),
        ),
    ];

    if globals.json {
        io::out(&serde_json::to_string_pretty(&json!({
            "ok": true,
            "endpoint": client.transport().endpoint(),
            "browser": link,
            "state": state,
            "tools": offered,
        }))?);
    } else {
        io::out(&render::fields(&rows));
        if !offered.is_empty() {
            io::out("");
            let optional = capability::OPTIONAL
                .iter()
                .map(|entry| {
                    let verdict = if offered.iter().any(|name| name == entry.tool) {
                        "offered".to_string()
                    } else {
                        format!("not offered ({})", entry.absent_because)
                    };
                    (entry.command, verdict)
                })
                .collect::<Vec<_>>();
            let rendered = optional
                .iter()
                .map(|(command, verdict)| (*command, verdict.clone()))
                .collect::<Vec<_>>();
            io::out("optional");
            io::out(
                &render::fields(&rendered)
                    .lines()
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    }
    Ok(match ready {
        Some((200, _)) => Code::Ok,
        Some((503, _)) => Code::NoBrowser,
        _ => Code::Ok,
    })
}

/// Names the task this directory is about to do. A task is its tab group and a
/// group needs a page, so the name waits for the first page and is applied then.
fn task_start(
    client: &mut Client,
    name: Option<String>,
    category: Option<String>,
    summary: Option<String>,
) -> Result<Code> {
    if let Some(group) = client.state().group_id.clone() {
        return Err(Failure::usage(format!(
            "this directory already has a task, in tab group {}.\n  Finish it first: browseros task done",
            client::short(&group)
        ))
        .into());
    }
    let pending = name.clone().map(|name| state::PendingTask {
        name,
        category,
        summary,
    });
    client.persist(|state| {
        state.pending_task = pending;
        state.started_at = Some(state::now_stamp());
    })?;
    io::out(&render::fields(&[
        ("task", name.unwrap_or_else(|| "(unnamed)".to_string())),
        ("group", "opens with the first page".to_string()),
    ]));
    io::out(
        "\nEvery command in this directory joins this task.\nWhen the user says it is done: browseros task done",
    );
    Ok(Code::Ok)
}

/// Settles a name given before the first page, now that the task has a group. The name itself
/// went with the call that created the task; a category or summary still needs `name_session`.
async fn name_pending_task(client: &mut Client, pending: state::PendingTask) -> Result<()> {
    if pending.category.is_some() || pending.summary.is_some() {
        let arguments = client::object(vec![
            ("name", json!(pending.name)),
            (
                "category",
                pending.category.map_or(Value::Null, Value::String),
            ),
            (
                "summary",
                pending.summary.map_or(Value::Null, Value::String),
            ),
        ]);
        let named = client.call("name_session", arguments).await?;
        if named.is_error {
            io::note(&format!("[describing the task failed: {}]", named.text));
        }
    }
    client.persist(|state| {
        state.task = Some(pending.name);
        state.pending_task = None;
    })
}

fn task_list() {
    let tasks = state::all_scopes()
        .into_iter()
        .filter(|(_, value)| value.group_id.is_some() || value.pending_task.is_some())
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        io::out("(no tasks on this machine)");
        return;
    }
    for (key, value) in tasks {
        let group = value
            .group_id
            .as_deref()
            .map(client::short)
            .unwrap_or_else(|| "-".to_string());
        let name = value
            .task
            .or_else(|| value.pending_task.map(|pending| pending.name))
            .unwrap_or_default();
        io::out(&format!(
            "{key}  {group:<10} {name:<24} {}",
            value.scope_path.unwrap_or_default()
        ));
    }
}

/// The user said the task is done. Closing its tab group ends it on the server,
/// so there is nothing else to release. `keep_tabs` leaves the pages open and
/// only takes them out of the group, which ends the task the same way.
async fn task_done(client: &mut Client, keep_tabs: bool, all: bool) -> Result<Code> {
    match client.state().group_id.clone() {
        None => io::out("no task in this directory."),
        Some(group) if keep_tabs => {
            let listed = client.call("tabs", json!({ "action": "list" })).await?;
            let pages = render::pages_in_group(&listed, &group);
            if !pages.is_empty() {
                let arguments = json!({ "action": "ungroup", "pages": pages });
                let ungrouped = client.call("tab_groups", arguments).await?;
                if ungrouped.is_error {
                    io::note(&format!("[{}]", ungrouped.text));
                }
            }
            io::out("task done. Its pages are still open, out of their group.");
        }
        Some(group) => {
            let arguments = json!({ "action": "close", "groupId": group });
            let closed = client.call("tab_groups", arguments).await?;
            io::out(if closed.is_error {
                "task done. Its tab group was already closed."
            } else {
                "task done. Its tab group is closed."
            });
        }
    }
    client.store().clear()?;

    if all {
        let mut cleared = 0;
        if let Some(dir) = state::state_dir()
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && std::fs::remove_file(&path).is_ok()
                {
                    cleared += 1;
                }
            }
        }
        io::out(&format!("cleared {cleared} scope file(s)."));
    }
    Ok(Code::Ok)
}

/// Reads a saved output part on to its end after its first chunk, so the whole part prints
/// as one text. Each chunk keeps the server's untrusted-content markers; only the cursor line
/// it leads with is dropped, since the CLI has done the paging it describes. Returns how many
/// chunks were read.
///
/// # Errors
/// Propagates a failed call; a chunk the server refuses replaces the output with its error.
pub async fn read_rest_of_output(
    client: &mut Client,
    args: &serde_json::Map<String, Value>,
    output: &mut ToolOutput,
) -> Result<usize> {
    let mut texts = vec![without_output_cursor(&output.text).to_string()];
    let mut next = next_output_offset(output);
    while let Some(offset) = next {
        let mut chunk_args = args.clone();
        chunk_args.insert("offset".to_string(), json!(offset));
        let chunk = client
            .call("read_output", Value::Object(chunk_args))
            .await?;
        if chunk.is_error {
            *output = chunk;
            return Ok(texts.len());
        }
        texts.push(without_output_cursor(&chunk.text).to_string());
        next = next_output_offset(&chunk);
    }
    output.text = texts.join("\n");
    Ok(texts.len())
}

/// For a slice the caller asked for, says how much of the part is left and how to read it.
fn note_rest_of_output(args: &serde_json::Map<String, Value>, output: &ToolOutput) {
    let Some(next) = next_output_offset(output) else {
        return;
    };
    let total = output
        .structured
        .as_ref()
        .and_then(|value| value.get("totalBytes"))
        .and_then(Value::as_u64)
        .unwrap_or(next);
    let id = args
        .get("outputId")
        .and_then(Value::as_str)
        .unwrap_or("<id>");
    let part = args.get("part").and_then(Value::as_str).unwrap_or("<part>");
    io::note(&format!(
        "[{} more bytes: browseros output {id} --part {part} --offset {next}]",
        total.saturating_sub(next)
    ));
}

fn next_output_offset(output: &ToolOutput) -> Option<u64> {
    let structured = output.structured.as_ref()?;
    if structured
        .get("eof")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        return None;
    }
    structured.get("nextOffset").and_then(Value::as_u64)
}

fn without_output_cursor(text: &str) -> &str {
    text.strip_prefix("Output chunk: ")
        .and_then(|rest| rest.split_once('\n'))
        .map_or(text, |(_, body)| body)
}

fn report(
    output: &ToolOutput,
    label: &'static str,
    tool: &'static str,
    page: Option<u32>,
    elapsed: u128,
    files: &[String],
    globals: &Globals,
) {
    for notice in &output.notices {
        io::note(notice);
    }
    if globals.json {
        io::out(&render::envelope(label, tool, page, output, elapsed, files));
        return;
    }
    if !output.text.is_empty() {
        io::out(&output.text);
    }
    for file in files {
        io::out(file);
    }
}

/// Writes inline image blocks to disk, because a terminal cannot render one.
fn write_images(output: &ToolOutput, target: Option<&str>) -> Result<Vec<String>> {
    if output.images.is_empty() {
        return Ok(Vec::new());
    }
    let dir = default_output_dir();
    std::fs::create_dir_all(&dir)?;
    let mut written = Vec::new();
    for (index, image) in output.images.iter().enumerate() {
        let extension = match image.mime_type.as_str() {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        };
        let path = match (target, index) {
            (Some(target), 0) => std::path::PathBuf::from(target),
            (Some(target), n) => std::path::PathBuf::from(format!("{target}.{n}")),
            (None, _) => dir.join(format!(
                "screenshot-{}-{index}.{extension}",
                std::process::id()
            )),
        };
        let bytes = decode_base64(&image.data)?;
        std::fs::write(&path, bytes)?;
        written.push(path.display().to_string());
    }
    Ok(written)
}

fn default_output_dir() -> std::path::PathBuf {
    if let Some(raw) = std::env::var_os("BROWSEROS_DIR").filter(|value| !value.is_empty()) {
        return std::path::PathBuf::from(raw).join("tool-output");
    }
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".browseros")
        .join("tool-output")
}

/// Standard base64, decoded by hand rather than adding a dependency for the
/// one place the CLI needs it.
fn decode_base64(encoded: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = u8::try_from(index).unwrap_or(0);
    }
    let mut out = Vec::with_capacity(encoded.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in encoded.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return Err(Failure::tool("the server returned an unreadable image").into());
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xFF).unwrap_or(0));
        }
    }
    Ok(out)
}

/// On a page-scoped failure, say precisely whether the tab is still there.
async fn explain_stale_page(client: &mut Client, page: Option<u32>) {
    let Some(page) = page else { return };
    let Ok(listed) = client.call("tabs", json!({ "action": "list" })).await else {
        return;
    };
    let listed_pages = render::listed_pages(&listed);
    // An unreadable or empty listing must not be read as "the tab is gone":
    // clearing the remembered page on every tool failure is worse than never
    // clearing it, because the next command then fails for a different reason.
    if listed_pages.is_empty() || listed_pages.contains(&page) {
        return;
    }
    io::note(&format!(
        "[page {page} is no longer open, so the remembered page has been cleared.\n Pick another: browseros tab list, then browseros use <id>]"
    ));
    let _ = client.persist(|state| {
        state.page = None;
        state.page_url = None;
    });
}

fn read_external(source: &External) -> Result<String> {
    match source {
        External::Stdin => {
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            Ok(buffer)
        }
        External::File(path) => std::fs::read_to_string(path)
            .map_err(|error| Failure::usage(format!("cannot read {path}: {error}")).into()),
        External::Env(name) => std::env::var(name)
            .map_err(|_| Failure::usage(format!("{name} is not set in the environment")).into()),
    }
}

fn command_label(command: &Command) -> &'static str {
    match command {
        Command::Status => "status",
        Command::Task(_) => "task",
        Command::Use { .. } => "use",
        Command::Open { .. } => "open",
        Command::Goto { .. } => "goto",
        Command::Back => "back",
        Command::Forward => "forward",
        Command::Reload => "reload",
        Command::History { .. } => "history",
        Command::Tab(_) => "tab",
        Command::Group(_) => "group",
        Command::Window(_) => "window",
        Command::Profile(_) => "profile",
        Command::Target(_) => "target",
        Command::Output { .. } => "output",
        Command::Credentials(_) => "credentials",
        Command::Cards(_) => "cards",
        Command::Snapshot { .. } => "snapshot",
        Command::Diff => "diff",
        Command::Read { .. } => "read",
        Command::Grep { .. } => "grep",
        Command::Screenshot { .. } => "screenshot",
        Command::Pdf { .. } => "pdf",
        Command::Click { .. } => "click",
        Command::Fill { .. } => "fill",
        Command::Type { .. } => "type",
        Command::Press { .. } => "press",
        Command::Hover { .. } => "hover",
        Command::Focus { .. } => "focus",
        Command::Check { .. } => "check",
        Command::Uncheck { .. } => "uncheck",
        Command::Select { .. } => "select",
        Command::Scroll { .. } => "scroll",
        Command::Drag { .. } => "drag",
        Command::Dialog(_) => "dialog",
        Command::Upload { .. } => "upload",
        Command::Download { .. } => "download",
        Command::Wait { .. } => "wait",
        Command::Run { .. } => "run",
        Command::Eval { .. } => "eval",
        Command::Helpers(_) => "helpers",
        Command::Skill(_) => "skill",
        Command::Help(_) => "help",
        Command::Pursue { .. } => "pursue",
        Command::Guide { .. } => "guide",
        Command::InstallCli { .. } => "install-cli",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_the_bytes_a_screenshot_would_carry() -> Result<()> {
        assert_eq!(decode_base64("aGVsbG8=")?, b"hello");
        assert_eq!(decode_base64("//79")?, vec![0xFF, 0xFE, 0xFD]);
        assert!(decode_base64("not base64!").is_err());
        Ok(())
    }

    #[test]
    fn whitespace_inside_a_base64_payload_is_ignored() -> Result<()> {
        assert_eq!(decode_base64("aGVs\nbG8=")?, b"hello");
        Ok(())
    }
}
