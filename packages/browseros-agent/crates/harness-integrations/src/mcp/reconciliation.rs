//! Conservative MCP maintenance: creation uses the normal ownership pipeline;
//! existing entries retain every field except a recognized endpoint's port.
//! Format adapters edit one value, so user options and comments survive repair.
use std::{collections::BTreeMap, path::Path, str::FromStr};

use jsonc_parser::{ParseOptions, cst::CstRootNode};
use serde_json::Value as JsonValue;
use toml_edit::{DocumentMut, Value};
use url::Url;

use super::{
    emitter::{Emitter, transform_key},
    io::{FsOp, Plan, apply_plan, read_state},
    paths::resolve_agent_surface,
    planner::plan_link,
    types::{
        AgentScope, AgentSurface, DisconnectInput, DisconnectSummary, LinkInput, McpServerSpec,
        ReconcileInput, ReconcileSummary,
    },
};
use crate::{ConfigFormat, Error};

pub(super) fn reconcile(
    workspace: &Path,
    input: ReconcileInput,
) -> Result<ReconcileSummary, Error> {
    let initial = read_state(workspace, &[], AgentScope::System, &BTreeMap::new())?;
    let names = std::iter::once(input.server.name.as_str())
        .chain(input.aliases.iter().map(String::as_str))
        .collect::<Vec<_>>();
    let recorded = names.iter().find_map(|name| {
        initial
            .manifest
            .servers
            .get(*name)
            .and_then(|server| server.links.get(&input.agent))
            .map(|link| link.config_path.clone())
    });
    let path = input.config_path.clone().or(recorded);
    let overrides = path
        .clone()
        .map(|path| BTreeMap::from([(input.agent, path)]))
        .unwrap_or_default();
    let state = read_state(workspace, &[input.agent], AgentScope::System, &overrides)?;
    let file = &state.agents[0];
    let surface = resolve_agent_surface(input.agent, AgentScope::System)?;
    let target = target_url(&input.server.spec)?;
    let edited = edit_existing(&file.raw_content, &names, &surface, &target)?;
    let (names, next_raw, created) = match edited {
        Existing::Missing => (vec![input.server.name.clone()], None, true),
        Existing::Foreign => return Ok(ReconcileSummary::default()),
        Existing::Recognized { names, raw } => (names, Some(raw), false),
    };
    let updated = next_raw
        .as_ref()
        .is_some_and(|raw| raw != &file.raw_content);
    let linked = names.iter().all(|name| {
        state
            .manifest
            .servers
            .get(name)
            .and_then(|server| server.links.get(&input.agent))
            .is_some_and(|link| link.config_path == file.config_path)
    });
    if !created && !updated && linked {
        return Ok(ReconcileSummary {
            connected: true,
            ..ReconcileSummary::default()
        });
    }
    let mut link = LinkInput::new(input.server, input.agent);
    link.config_path = path;
    // Only a recognized endpoint reaches adoption. Unknown entries never reach
    // the overwrite-capable planner; explicit connection still has its own rules.
    link.allow_overwrite = !created;
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| invalid(error.to_string()))?;
    // A profile can contain multiple historic names. Adopt every recognized
    // endpoint in one manifest update and write the combined port edits once;
    // a foreign canonical entry must not hide a valid local alias.
    let mut planning = state.clone();
    let mut plan = Plan {
        ops: Vec::new(),
        next_manifest: state.manifest.clone(),
    };
    for name in names {
        link.server.name = name;
        plan = plan_link(&planning, &link, &now)?.plan;
        planning.manifest = plan.next_manifest.clone();
    }
    if let Some(next_raw) = next_raw {
        plan.ops
            .retain(|op| !matches!(op, FsOp::WriteFile { path, .. } if path == &file.config_path));
        if updated {
            plan.ops.insert(
                0,
                FsOp::WriteFile {
                    path: file.config_path.clone(),
                    content: next_raw,
                },
            );
        }
    }
    if plan.next_manifest == state.manifest {
        plan.ops.retain(
            |op| !matches!(op, FsOp::WriteFile { path, .. } if path == &state.manifest_path),
        );
    }
    apply_plan(&plan)?;
    Ok(ReconcileSummary {
        connected: true,
        created,
        updated,
    })
}

/// Explicit removal can recognize an older/manual installation without first
/// rewriting or adopting it. The endpoint identity check is shared with repair.
pub(super) fn disconnect_unrecorded(
    workspace: &Path,
    input: DisconnectInput,
) -> Result<DisconnectSummary, Error> {
    let overrides = input
        .config_path
        .clone()
        .map(|path| BTreeMap::from([(input.agent, path)]))
        .unwrap_or_default();
    let state = read_state(workspace, &[input.agent], input.scope, &overrides)?;
    let file = &state.agents[0];
    let mut summary = DisconnectSummary {
        server_name: input.server_name.clone(),
        agent: input.agent,
        scope: input.scope,
        unlinked: false,
        removed_manifest: false,
    };
    let Some(endpoint) = &input.unmanaged_endpoint else {
        return Ok(summary);
    };
    let target = Url::parse(endpoint).map_err(|error| invalid(error.to_string()))?;
    let surface = resolve_agent_surface(input.agent, input.scope)?;
    if matches!(
        edit_existing(&file.raw_content, &[&input.server_name], &surface, &target)?,
        Existing::Recognized { .. }
    ) {
        let next = Emitter::new(surface).remove(&file.raw_content, &input.server_name)?;
        apply_plan(&Plan {
            ops: vec![FsOp::WriteFile {
                path: file.config_path.clone(),
                content: next,
            }],
            next_manifest: state.manifest,
        })?;
        summary.unlinked = true;
    }
    Ok(summary)
}

enum Existing {
    Missing,
    Foreign,
    Recognized { names: Vec<String>, raw: String },
}

fn target_url(spec: &McpServerSpec) -> Result<Url, Error> {
    let raw = match spec {
        McpServerSpec::Http { url, .. } | McpServerSpec::Sse { url, .. } => url,
        McpServerSpec::Stdio { command, args, .. }
            if command == "npx" && args.first().is_some_and(|arg| arg == "mcp-remote") =>
        {
            args.get(1)
                .ok_or_else(|| invalid("mcp-remote endpoint missing"))?
        }
        McpServerSpec::Stdio { .. } => {
            return Err(invalid("port reconciliation requires a URL endpoint"));
        }
    };
    Url::parse(raw).map_err(|error| invalid(error.to_string()))
}

/// Identifies just the editable string; unrelated/unknown options stay opaque.
struct EndpointSlot {
    field: &'static str,
    index: Option<usize>,
    url: String,
}

fn endpoint_slot(
    surface: &AgentSurface,
    mut read: impl FnMut(&str) -> Option<JsonValue>,
) -> Option<EndpointSlot> {
    if let Some(http) = surface.http {
        let field = http.url_field.unwrap_or("url");
        if let Some(JsonValue::String(url)) = read(field) {
            return Some(EndpointSlot {
                field,
                index: None,
                url,
            });
        }
    }
    let command = surface.stdio.command_field.unwrap_or("command");
    let (field, index) = if surface.stdio.command_as_array {
        (command, 2)
    } else {
        if read(command)?.as_str()? != "npx" {
            return None;
        }
        (surface.stdio.args_field.unwrap_or("args"), 1)
    };
    let parts = read(field)?;
    let parts = parts.as_array()?;
    if surface.stdio.command_as_array && parts.first()?.as_str()? != "npx" {
        return None;
    }
    if parts.get(index - 1)?.as_str()? != "mcp-remote" {
        return None;
    }
    Some(EndpointSlot {
        field,
        index: Some(index),
        url: parts.get(index)?.as_str()?.to_string(),
    })
}

fn new_endpoint(raw: &str, target: &Url) -> Option<String> {
    let mut actual = Url::parse(raw).ok()?;
    // Never redirect a customized host, path, scheme or identity. Queries and
    // fragments belong to the existing endpoint and survive a port change.
    if actual.scheme() != target.scheme()
        || actual.host_str() != target.host_str()
        || actual.path() != target.path()
        || actual.username() != target.username()
        || actual.password() != target.password()
    {
        return None;
    }
    if actual.port_or_known_default() == target.port_or_known_default() {
        return Some(raw.to_string());
    }
    actual.set_port(target.port()).ok()?;
    Some(actual.to_string())
}

fn edit_existing(
    raw: &str,
    names: &[&str],
    surface: &AgentSurface,
    target: &Url,
) -> Result<Existing, Error> {
    if raw.trim().is_empty() {
        return Ok(Existing::Missing);
    }
    match surface.mcp.format {
        ConfigFormat::Json | ConfigFormat::Jsonc => edit_json(raw, names, surface, target),
        ConfigFormat::Toml => edit_toml(raw, names, surface, target),
    }
}

fn edit_json(
    raw: &str,
    names: &[&str],
    surface: &AgentSurface,
    target: &Url,
) -> Result<Existing, Error> {
    let root = CstRootNode::parse(raw, &ParseOptions::default())
        .map_err(|error| invalid(error.to_string()))?;
    let object = root
        .object_value()
        .ok_or_else(|| invalid("MCP config root is not an object"))?;
    let Some(container) = object.get(surface.stdio.top_level_key) else {
        return Ok(Existing::Missing);
    };
    let container = container
        .value()
        .and_then(|value| value.as_object())
        .ok_or_else(|| invalid("MCP server collection is not an object"))?;
    let canonical_present = container
        .get(&transform_key(names[0], surface.stdio))
        .is_some();
    let mut recognized = Vec::new();
    let mut updated = false;
    for name in names {
        let key = transform_key(name, surface.stdio);
        let Some(property) = container.get(&key) else {
            continue;
        };
        let Some(entry) = property.value().and_then(|value| value.as_object()) else {
            continue;
        };
        let Some(slot) =
            endpoint_slot(surface, |field| entry.get(field)?.value()?.to_serde_value())
        else {
            continue;
        };
        let Some(next) = new_endpoint(&slot.url, target) else {
            continue;
        };
        recognized.push((*name).to_string());
        if next == slot.url {
            continue;
        }
        updated = true;
        let node = entry
            .get(slot.field)
            .and_then(|property| property.value())
            .ok_or_else(|| invalid("endpoint disappeared"))?;
        let node = match slot.index {
            Some(index) => node
                .as_array()
                .and_then(|array| array.elements().get(index).cloned())
                .ok_or_else(|| invalid("endpoint array changed"))?,
            None => node,
        };
        node.as_string_lit()
            .ok_or_else(|| invalid("endpoint is not a string"))?
            .set_raw_value(
                serde_json::to_string(&next).map_err(|error| invalid(error.to_string()))?,
            );
    }
    Ok(edited_entries(
        recognized,
        canonical_present,
        if updated {
            root.to_string()
        } else {
            raw.to_string()
        },
    ))
}

fn edit_toml(
    raw: &str,
    names: &[&str],
    surface: &AgentSurface,
    target: &Url,
) -> Result<Existing, Error> {
    let mut document = DocumentMut::from_str(raw).map_err(|error| invalid(error.to_string()))?;
    let Some(container) = document.get_mut(surface.stdio.top_level_key) else {
        return Ok(Existing::Missing);
    };
    let container = container
        .as_table_like_mut()
        .ok_or_else(|| invalid("MCP server collection is not a table"))?;
    let canonical_present = container
        .get(&transform_key(names[0], surface.stdio))
        .is_some();
    let mut recognized = Vec::new();
    let mut updated = false;
    for name in names {
        let key = transform_key(name, surface.stdio);
        let Some(entry) = container.get_mut(&key) else {
            continue;
        };
        let Some(entry) = entry.as_table_like_mut() else {
            continue;
        };
        let Some(slot) = endpoint_slot(surface, |field| {
            let value = entry.get(field)?;
            if let Some(value) = value.as_str() {
                Some(JsonValue::String(value.to_string()))
            } else {
                Some(JsonValue::Array(
                    value
                        .as_array()?
                        .iter()
                        .map(|value| value.as_str().map(|s| JsonValue::String(s.to_string())))
                        .collect::<Option<Vec<_>>>()?,
                ))
            }
        }) else {
            continue;
        };
        let Some(next) = new_endpoint(&slot.url, target) else {
            continue;
        };
        recognized.push((*name).to_string());
        if next == slot.url {
            continue;
        }
        updated = true;
        let field = entry
            .get_mut(slot.field)
            .ok_or_else(|| invalid("endpoint disappeared"))?;
        let value = match slot.index {
            Some(index) => field.as_array_mut().and_then(|array| array.get_mut(index)),
            None => field.as_value_mut(),
        }
        .ok_or_else(|| invalid("endpoint is not a value"))?;
        let decor = value.decor().clone();
        *value = Value::from(next);
        *value.decor_mut() = decor;
    }
    Ok(edited_entries(
        recognized,
        canonical_present,
        if updated {
            document.to_string()
        } else {
            raw.to_string()
        },
    ))
}

fn edited_entries(names: Vec<String>, canonical_present: bool, raw: String) -> Existing {
    if !names.is_empty() {
        Existing::Recognized { names, raw }
    } else if canonical_present {
        Existing::Foreign
    } else {
        Existing::Missing
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidServerSpec {
        reason: message.into(),
    }
}
