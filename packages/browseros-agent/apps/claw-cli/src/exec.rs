//! Running a plan: resolve what only state knows, call, then print and exit.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};

use crate::cli::{Cli, Command, Globals};
use crate::client::{self, Session, result::ToolOutput};
use crate::exit::{Code, Failure};
use crate::plan::{self, External, Local, Plan, ToolPlan};
use crate::{capability, guides, install, io, render, script, state};

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
    let mut session =
        client::connect(cli.globals.endpoint.as_deref(), agent.as_deref(), timeout).await?;

    match built {
        Plan::Tool(tool) => run_tool(&mut session, &cli.globals, label, *tool).await,
        Plan::Local(local) => run_local(&mut session, &cli.globals, local).await,
    }
}

async fn run_tool(
    session: &mut Session,
    globals: &Globals,
    label: &'static str,
    mut plan: ToolPlan,
) -> Result<Code> {
    for notice in &plan.notices {
        io::note(notice);
    }

    let page = if plan.needs_page {
        let page = globals
            .page
            .or(session.state().page)
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
        && let Some(page) = globals.page.or(session.state().page)
        && let Some(Value::String(code)) = plan.args.get("code")
    {
        let wrapped = script::with_page(code, page);
        plan.args.insert("code".to_string(), Value::String(wrapped));
    }

    if plan.with_group
        && let Some(group) = session.state().group_id.clone()
    {
        plan.args
            .entry("groupId".to_string())
            .or_insert(Value::String(group));
    }

    if let Some(reason) = capability::unavailable(plan.tool, &session.state().tools) {
        return Err(Failure::tool(format!(
            "this server does not offer `{}`: {reason}.\n  Run `browseros status` to see what it does offer.",
            plan.tool
        ))
        .into());
    }

    let started = Instant::now();
    let arguments = Value::Object(plan.args.clone());
    let output = session
        .call(plan.tool, arguments, !globals.no_auto_session)
        .await?;
    let elapsed = started.elapsed().as_millis();

    if plan.remember_page
        && let Some(new_page) = render::page_from(&output)
    {
        let url = output
            .structured
            .as_ref()
            .and_then(|value| value.get("url"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        session.persist(|state| {
            state.page = Some(new_page);
            state.page_url = url.clone();
        })?;
    }

    let mut files = Vec::new();
    if let Some(target) = plan.save_image.clone() {
        files = write_images(&output, target.as_deref())?;
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
        io::note(script::CAP_ADVICE);
        return Ok(Code::Timeout);
    }

    report(&output, label, plan.tool, page, elapsed, &files, globals);

    // A page-scoped failure is usually a closed or recycled tab, and the one
    // extra round trip to say so precisely only happens on the failure path.
    if output.is_error && plan.needs_page {
        explain_stale_page(session, page).await;
    }
    Ok(output.exit_code())
}

/// The locals that need neither the network nor a session.
fn run_offline(local: &Local) -> Result<Option<Code>> {
    match local {
        Local::SkillsList => {
            io::out(&guides::list());
            Ok(Some(Code::Ok))
        }
        Local::SkillsGet { name, full } => match guides::get(name, *full) {
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
        Local::SessionList => {
            session_list();
            Ok(Some(Code::Ok))
        }
        Local::InstallCli { confirm } => {
            let path = std::env::var("PATH").ok();
            install::run(*confirm, path.as_deref()).map(Some)
        }
        _ => Ok(None),
    }
}

async fn run_local(session: &mut Session, globals: &Globals, local: Local) -> Result<Code> {
    match local {
        Local::Status => status(session, globals).await,
        Local::Use(page) => {
            session.persist(|state| {
                state.page = Some(page);
                state.page_url = None;
            })?;
            io::out(&format!("page {page}"));
            Ok(Code::Ok)
        }
        Local::SessionStart {
            task,
            category,
            summary,
        } => session_start(session, globals, task, category, summary).await,
        Local::SessionEnd { close_tabs, all } => session_end(session, close_tabs, all).await,
        // Already answered by run_offline before the server was contacted.
        Local::SkillsList
        | Local::SkillsGet { .. }
        | Local::SessionList
        | Local::InstallCli { .. } => Ok(Code::Ok),
    }
}

async fn status(session: &mut Session, globals: &Globals) -> Result<Code> {
    let ready = session.transport().ready().await;
    let link = client::link_summary(ready.clone());
    // One tools/list, cached against the server version. It is the largest
    // response the CLI ever reads, so asking once per pairing rather than per
    // command is the whole cost argument for a CLI over MCP.
    let offered = session.refresh_tools().await.unwrap_or_default();
    let state = session.state().clone();

    let rows = vec![
        (
            "endpoint",
            format!("{}  ({link})", session.transport().endpoint()),
        ),
        (
            "session",
            state
                .mcp_session_id
                .as_deref()
                .map(client::short)
                .unwrap_or_else(|| "not paired".to_string()),
        ),
        ("task", state.task.clone().unwrap_or_default()),
        (
            "group",
            state
                .group_id
                .as_deref()
                .map(client::short)
                .unwrap_or_default(),
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
            "endpoint": session.transport().endpoint(),
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

async fn session_start(
    session: &mut Session,
    globals: &Globals,
    task: Option<String>,
    category: Option<String>,
    summary: Option<String>,
) -> Result<Code> {
    session.handshake(false).await?;

    // The server's own guidance prescribes a list on the first call of a
    // connection, and it is how a group left behind by a previous session for
    // this scope is adopted rather than a second one being started.
    let listed = session
        .call(
            "tabs",
            json!({ "action": "list" }),
            !globals.no_auto_session,
        )
        .await?;
    let adopted = session.state().group_id.clone().or_else(|| {
        listed
            .structured
            .as_ref()
            .and_then(|value| value.get("pages"))
            .and_then(Value::as_array)
            .and_then(|pages| {
                pages
                    .iter()
                    .find_map(|page| page.get("groupId").and_then(Value::as_str))
            })
            .map(ToString::to_string)
    });

    if let Some(name) = task.clone() {
        let arguments = client::object(vec![
            ("name", json!(name)),
            ("category", category.map_or(Value::Null, Value::String)),
            ("summary", summary.map_or(Value::Null, Value::String)),
        ]);
        let named = session.call("name_session", arguments, true).await?;
        if named.is_error {
            io::note(&format!("[name_session: {}]", named.text));
        }
    }

    let task_for_state = task.clone();
    let group_for_state = adopted.clone();
    session.persist(|state| {
        state.task = task_for_state.clone();
        state.group_id = group_for_state.clone();
    })?;

    let id = session
        .state()
        .mcp_session_id
        .as_deref()
        .map(client::short)
        .unwrap_or_default();
    io::out(&render::fields(&[
        (
            "session",
            format!("{id}  started on {}", session.transport().endpoint()),
        ),
        ("task", task.unwrap_or_default()),
        (
            "group",
            adopted
                .as_deref()
                .map(|group| format!("{}  (adopted)", client::short(group)))
                .unwrap_or_else(|| "none yet".to_string()),
        ),
    ]));
    io::out(
        "\nLater commands in this directory reuse this session.\nEnd it with: browseros session end",
    );
    Ok(Code::Ok)
}

fn session_list() {
    let scopes = state::all_scopes();
    if scopes.is_empty() {
        io::out("(no sessions on this machine)");
        return;
    }
    for (key, value) in scopes {
        let id = value
            .mcp_session_id
            .as_deref()
            .map(client::short)
            .unwrap_or_else(|| "-".to_string());
        io::out(&format!(
            "{key}  {id:<10} {:<24} {}",
            value.task.unwrap_or_default(),
            value.scope_path.unwrap_or_default()
        ));
    }
}

async fn session_end(session: &mut Session, close_tabs: bool, all: bool) -> Result<Code> {
    // Closing the group is a decision about the user's browser, so it is a
    // flag. Leaving tabs open matches the guidance that useful pages should
    // survive the end of a task unless asked otherwise.
    if close_tabs {
        if let Some(group) = session.state().group_id.clone() {
            let arguments = json!({ "action": "close", "groupId": group });
            let closed = session.call("tab_groups", arguments, false).await?;
            io::out(&closed.text);
        } else {
            io::note("[no tab group recorded for this directory; nothing was closed]");
        }
    }

    let status = session.release().await?;
    session.store().clear()?;
    io::out(match status {
        Some(202) => "session released.",
        Some(_) => "session released (the server had already forgotten it).",
        None => "no session in this directory.",
    });

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

fn report(
    output: &ToolOutput,
    label: &'static str,
    tool: &'static str,
    page: Option<u32>,
    elapsed: u128,
    files: &[String],
    globals: &Globals,
) {
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
async fn explain_stale_page(session: &mut Session, page: Option<u32>) {
    let Some(page) = page else { return };
    let Ok(listed) = session
        .call("tabs", json!({ "action": "list" }), false)
        .await
    else {
        return;
    };
    let open = listed
        .structured
        .as_ref()
        .and_then(|value| value.get("pages"))
        .and_then(Value::as_array)
        .is_some_and(|pages| {
            pages
                .iter()
                .any(|entry| entry.get("page").and_then(Value::as_u64) == Some(u64::from(page)))
        });
    if open {
        return;
    }
    io::note(&format!(
        "[page {page} is no longer open, so the remembered page has been cleared.\n Pick another: browseros tab list, then browseros use <id>]"
    ));
    let _ = session.persist(|state| {
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
        Command::Session(_) => "session",
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
        Command::Skills(_) => "skills",
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
