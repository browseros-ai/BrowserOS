use std::{collections::BTreeMap, str::FromStr};

use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstObject, CstRootNode},
};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value, value as toml_value};

use crate::{
    AgentId,
    catalog::{ConfigFormat, HttpShape, InjectValue, KeyTransform, StdioShape},
    error::Error,
};

use super::types::{AgentSurface, McpServerSpec};

const UTF8_BOM: char = '\u{feff}';

pub(crate) struct Emitter {
    surface: AgentSurface,
}

pub(super) fn split_bom(raw: &str) -> (bool, &str) {
    match raw.strip_prefix(UTF8_BOM) {
        Some(content) => (true, content),
        None => (false, raw),
    }
}

pub(super) fn with_bom(content: String, had_bom: bool) -> String {
    if had_bom {
        format!("{UTF8_BOM}{content}")
    } else {
        content
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryValue {
    String(String),
    Bool(bool),
    Strings(Vec<String>),
    StringMap(BTreeMap<String, String>),
}

impl Emitter {
    pub(crate) const fn new(surface: AgentSurface) -> Self {
        Self { surface }
    }

    pub(crate) fn read(&self, raw: &str) -> Result<Vec<String>, Error> {
        match self.surface.mcp.format {
            ConfigFormat::Json | ConfigFormat::Jsonc => Ok(json_read(raw, self.surface.stdio)),
            ConfigFormat::Toml => toml_read(raw, self.surface.stdio),
        }
    }

    pub(crate) fn inspect(&self, raw: &str, name: &str) -> Result<Option<McpServerSpec>, Error> {
        let key = transform_key(name, self.surface.stdio);
        let entry = match self.surface.mcp.format {
            ConfigFormat::Json | ConfigFormat::Jsonc => json_entry(raw, self.surface.stdio, &key)?,
            ConfigFormat::Toml => toml_entry(raw, self.surface.stdio.top_level_key, &key)?,
        };
        Ok(entry.and_then(|entry| decode_entry(&entry, self.surface.stdio, self.surface.http)))
    }

    pub(crate) fn add(&self, raw: &str, name: &str, spec: &McpServerSpec) -> Result<String, Error> {
        let key = transform_key(name, self.surface.stdio);
        let entry = build_entry(spec, self.surface.stdio, self.surface.http)?;
        match self.surface.mcp.format {
            ConfigFormat::Json | ConfigFormat::Jsonc => json_add(
                raw,
                self.surface.stdio,
                self.surface.harness.id,
                &key,
                entry,
            ),
            ConfigFormat::Toml => toml_add(raw, self.surface.stdio.top_level_key, &key, entry),
        }
    }

    pub(crate) fn remove(&self, raw: &str, name: &str) -> Result<String, Error> {
        let key = transform_key(name, self.surface.stdio);
        match self.surface.mcp.format {
            ConfigFormat::Json | ConfigFormat::Jsonc => json_remove(raw, self.surface.stdio, &key),
            ConfigFormat::Toml => toml_remove(raw, self.surface.stdio.top_level_key, &key),
        }
    }
}

fn decode_entry(
    entry: &BTreeMap<String, EntryValue>,
    stdio: StdioShape,
    http: Option<HttpShape>,
) -> Option<McpServerSpec> {
    if let Some(http) = http {
        let url_field = http.url_field.unwrap_or("url");
        if let Some(EntryValue::String(url)) = entry.get(url_field) {
            let headers = match http.header_field.unwrap_or("headers") {
                field if !entry.contains_key(field) => BTreeMap::new(),
                field => match entry.get(field) {
                    Some(EntryValue::StringMap(headers)) => headers.clone(),
                    _ => return None,
                },
            };
            let is_sse = http
                .tag_key
                .zip(http.sse_tag_value)
                .is_some_and(|(key, value)| {
                    entry.get(key) == Some(&EntryValue::String(value.to_string()))
                });
            let spec = if is_sse {
                McpServerSpec::Sse {
                    url: url.clone(),
                    headers,
                }
            } else {
                McpServerSpec::Http {
                    url: url.clone(),
                    headers,
                }
            };
            if entry == &entry_map(build_entry(&spec, stdio, Some(http)).ok()?) {
                return Some(spec);
            }
        }
    }

    let command_field = stdio.command_field.unwrap_or("command");
    let (command, args) = if stdio.command_as_array {
        let EntryValue::Strings(parts) = entry.get(command_field)? else {
            return None;
        };
        let (command, args) = parts.split_first()?;
        (command.clone(), args.to_vec())
    } else {
        let EntryValue::String(command) = entry.get(command_field)? else {
            return None;
        };
        let args_field = stdio.args_field.unwrap_or("args");
        let args = match entry.get(args_field) {
            None => Vec::new(),
            Some(EntryValue::Strings(args)) => args.clone(),
            _ => return None,
        };
        (command.clone(), args)
    };
    let env_field = stdio.env_field.unwrap_or("env");
    let env = match entry.get(env_field) {
        None => BTreeMap::new(),
        Some(EntryValue::StringMap(env)) => env.clone(),
        _ => return None,
    };
    let spec = McpServerSpec::Stdio { command, args, env };
    (entry == &entry_map(build_entry(&spec, stdio, http).ok()?)).then_some(spec)
}

fn entry_map(entry: Vec<(String, EntryValue)>) -> BTreeMap<String, EntryValue> {
    entry.into_iter().collect()
}

pub(super) fn transform_key(name: &str, shape: StdioShape) -> String {
    match shape.key_transform {
        Some(KeyTransform::SimpleName) => name
            .to_ascii_lowercase()
            .chars()
            .filter(char::is_ascii_alphabetic)
            .collect(),
        None => name.to_string(),
    }
}

fn build_entry(
    spec: &McpServerSpec,
    stdio: StdioShape,
    http: Option<HttpShape>,
) -> Result<Vec<(String, EntryValue)>, Error> {
    match spec {
        McpServerSpec::Stdio { command, args, env } => {
            Ok(build_stdio_entry(command, args, env, stdio))
        }
        McpServerSpec::Sse { url, headers } => {
            let http = http.ok_or_else(|| Error::InvalidServerSpec {
                reason: "client does not accept sse entries at this scope".to_string(),
            })?;
            Ok(build_http_entry(url, headers, true, http))
        }
        McpServerSpec::Http { url, headers } => {
            let http = http.ok_or_else(|| Error::InvalidServerSpec {
                reason: "client does not accept http entries at this scope".to_string(),
            })?;
            Ok(build_http_entry(url, headers, false, http))
        }
    }
}

fn build_stdio_entry(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    shape: StdioShape,
) -> Vec<(String, EntryValue)> {
    let command_field = shape.command_field.unwrap_or("command");
    let args_field = shape.args_field.unwrap_or("args");
    let env_field = shape.env_field.unwrap_or("env");
    let mut entry = Vec::new();
    if shape.command_as_array {
        let mut parts = Vec::with_capacity(args.len() + 1);
        parts.push(command.to_string());
        parts.extend_from_slice(args);
        entry.push((command_field.to_string(), EntryValue::Strings(parts)));
    } else {
        entry.push((
            command_field.to_string(),
            EntryValue::String(command.to_string()),
        ));
        if !args.is_empty() {
            entry.push((args_field.to_string(), EntryValue::Strings(args.to_vec())));
        }
    }
    if !env.is_empty() {
        entry.push((env_field.to_string(), EntryValue::StringMap(env.clone())));
    }
    append_injects(&mut entry, shape.injects);
    append_tag(&mut entry, shape.tag_key, shape.tag_value);
    entry
}

fn build_http_entry(
    url: &str,
    headers: &BTreeMap<String, String>,
    is_sse: bool,
    shape: HttpShape,
) -> Vec<(String, EntryValue)> {
    let mut entry = vec![(
        shape.url_field.unwrap_or("url").to_string(),
        EntryValue::String(url.to_string()),
    )];
    if !headers.is_empty() {
        entry.push((
            shape.header_field.unwrap_or("headers").to_string(),
            EntryValue::StringMap(headers.clone()),
        ));
    }
    append_injects(&mut entry, shape.injects);
    let tag_value = if is_sse {
        shape.sse_tag_value.or(shape.tag_value)
    } else {
        shape.tag_value
    };
    append_tag(&mut entry, shape.tag_key, tag_value);
    entry
}

fn append_injects(entry: &mut Vec<(String, EntryValue)>, injects: &[(&str, InjectValue)]) {
    for (key, value) in injects {
        let value = match value {
            InjectValue::String(value) => EntryValue::String((*value).to_string()),
            InjectValue::Bool(value) => EntryValue::Bool(*value),
        };
        entry.push(((*key).to_string(), value));
    }
}

fn append_tag(
    entry: &mut Vec<(String, EntryValue)>,
    tag_key: Option<&str>,
    tag_value: Option<&str>,
) {
    if let (Some(key), Some(value)) = (tag_key, tag_value) {
        entry.push((key.to_string(), EntryValue::String(value.to_string())));
    }
}

fn json_container(object: &CstObject, shape: StdioShape) -> Option<CstObject> {
    let section = object.object_value(shape.top_level_key)?;
    match shape.servers_key {
        Some(key) => section.object_value(key),
        None => Some(section),
    }
}

fn json_container_or_set(object: &CstObject, shape: StdioShape) -> CstObject {
    let section = object.object_value_or_set(shape.top_level_key);
    match shape.servers_key {
        Some(key) => section.object_value_or_set(key),
        None => section,
    }
}

/// Heal a server entry an older version placed directly under `top_level_key` (for example
/// OpenCode before it nested servers under `mcp.servers`). Once the harness nests entries
/// under `servers_key`, a leftover sibling at `top_level_key.<name>` is a stale duplicate
/// that some agents reject, so drop it. Never touches the servers container itself.
fn heal_legacy_entry(
    object: &CstObject,
    container: &CstObject,
    shape: StdioShape,
    agent: AgentId,
    name: &str,
) {
    // Self-healing of a pre-servers entry is OpenCode-specific. Gate it on the agent by name
    // so it never runs for any other harness, even one that later nests servers under the
    // same `servers` key.
    if agent != AgentId::OpenCode {
        return;
    }
    let Some(servers_key) = shape.servers_key else {
        return;
    };
    if name == servers_key {
        return;
    }
    let Some(section) = object.object_value(shape.top_level_key) else {
        return;
    };
    let Some(legacy) = section.get(name) else {
        return;
    };
    // Carry over a disable the user set on the old entry so moving it never silently
    // re-enables a server. OpenCode's pre-servers schema used `enabled`, the nested schema
    // uses `disabled`, so an `enabled: false` (or explicit `disabled: true`) on the old
    // entry becomes `disabled: true` on the new one.
    let was_disabled = legacy
        .value()
        .and_then(|value| value.to_serde_value())
        .map(|value| {
            value.get("disabled").and_then(serde_json::Value::as_bool) == Some(true)
                || value.get("enabled").and_then(serde_json::Value::as_bool) == Some(false)
        })
        .unwrap_or(false);
    if was_disabled
        && let Some(entry) = container.object_value(name)
        && entry.get("disabled").is_none()
    {
        entry.append("disabled", CstInputValue::Bool(true));
    }
    legacy.remove();
}

fn json_read(raw: &str, shape: StdioShape) -> Vec<String> {
    let (_, raw) = split_bom(raw);
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let Ok(root) = CstRootNode::parse(raw, &ParseOptions::default()) else {
        return Vec::new();
    };
    let Some(container) = root
        .object_value()
        .and_then(|object| json_container(&object, shape))
    else {
        return Vec::new();
    };
    container
        .properties()
        .into_iter()
        .filter_map(|property| property.name()?.decoded_value().ok())
        .collect()
}

fn json_entry(
    raw: &str,
    shape: StdioShape,
    name: &str,
) -> Result<Option<BTreeMap<String, EntryValue>>, Error> {
    let (_, raw) = split_bom(raw);
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let root =
        CstRootNode::parse(raw, &ParseOptions::default()).map_err(|error| Error::Config {
            format: "JSON/JSONC",
            message: error.to_string(),
        })?;
    let Some(value) = root
        .object_value()
        .and_then(|object| json_container(&object, shape))
        .and_then(|container| container.get(name))
        .and_then(|property| property.value())
        .and_then(|value| value.to_serde_value())
    else {
        return Ok(None);
    };
    Ok(json_entry_map(value))
}

fn json_entry_map(value: serde_json::Value) -> Option<BTreeMap<String, EntryValue>> {
    value
        .as_object()?
        .iter()
        .map(|(key, value)| json_entry_value(value.clone()).map(|value| (key.clone(), value)))
        .collect()
}

fn json_entry_value(value: serde_json::Value) -> Option<EntryValue> {
    match value {
        serde_json::Value::String(value) => Some(EntryValue::String(value)),
        serde_json::Value::Bool(value) => Some(EntryValue::Bool(value)),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(|value| value.as_str().map(ToString::to_string))
            .collect::<Option<Vec<_>>>()
            .map(EntryValue::Strings),
        serde_json::Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| value.as_str().map(|value| (key, value.to_string())))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(EntryValue::StringMap),
        _ => None,
    }
}

fn json_add(
    raw: &str,
    shape: StdioShape,
    agent: AgentId,
    name: &str,
    entry: Vec<(String, EntryValue)>,
) -> Result<String, Error> {
    let (had_bom, raw) = split_bom(raw);
    let seed = if raw.trim().is_empty() { "{}" } else { raw };
    let root =
        CstRootNode::parse(seed, &ParseOptions::default()).map_err(|error| Error::Config {
            format: "JSON/JSONC",
            message: error.to_string(),
        })?;
    let object = root.object_value_or_set();
    let container = json_container_or_set(&object, shape);
    let value = jsonc_value(entry);
    match container.get(name) {
        Some(property) => property.set_value(value),
        None => {
            container.append(name, value);
        }
    }
    heal_legacy_entry(&object, &container, shape, agent, name);
    Ok(with_bom(root.to_string(), had_bom))
}

fn json_remove(raw: &str, shape: StdioShape, name: &str) -> Result<String, Error> {
    let (had_bom, content) = split_bom(raw);
    if content.trim().is_empty() {
        return Ok(raw.to_string());
    }
    let root =
        CstRootNode::parse(content, &ParseOptions::default()).map_err(|error| Error::Config {
            format: "JSON/JSONC",
            message: error.to_string(),
        })?;
    if let Some(property) = root
        .object_value()
        .and_then(|object| json_container(&object, shape))
        .and_then(|container| container.get(name))
    {
        property.remove();
    }
    Ok(with_bom(root.to_string(), had_bom))
}

fn jsonc_value(entry: Vec<(String, EntryValue)>) -> CstInputValue {
    CstInputValue::Object(
        entry
            .into_iter()
            .map(|(key, value)| (key, jsonc_entry_value(value)))
            .collect(),
    )
}

fn jsonc_entry_value(value: EntryValue) -> CstInputValue {
    match value {
        EntryValue::String(value) => CstInputValue::String(value),
        EntryValue::Bool(value) => CstInputValue::Bool(value),
        EntryValue::Strings(values) => {
            CstInputValue::Array(values.into_iter().map(CstInputValue::String).collect())
        }
        EntryValue::StringMap(values) => CstInputValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, CstInputValue::String(value)))
                .collect(),
        ),
    }
}

fn parse_toml(raw: &str) -> Result<DocumentMut, Error> {
    if raw.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    DocumentMut::from_str(raw).map_err(|error| Error::Config {
        format: "TOML",
        message: error.to_string(),
    })
}

fn toml_read(raw: &str, shape: StdioShape) -> Result<Vec<String>, Error> {
    let document = parse_toml(raw)?;
    Ok(document
        .get(shape.top_level_key)
        .and_then(Item::as_table_like)
        .map(|table| table.iter().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default())
}

fn toml_entry(
    raw: &str,
    top_level_key: &str,
    name: &str,
) -> Result<Option<BTreeMap<String, EntryValue>>, Error> {
    let document = parse_toml(raw)?;
    let Some(entry) = document
        .get(top_level_key)
        .and_then(Item::as_table_like)
        .and_then(|table| table.get(name))
        .and_then(Item::as_table_like)
    else {
        return Ok(None);
    };
    Ok(entry
        .iter()
        .map(|(key, item)| decode_toml_entry_value(item).map(|value| (key.to_string(), value)))
        .collect())
}

fn decode_toml_entry_value(item: &Item) -> Option<EntryValue> {
    let value = item.as_value()?;
    if let Some(value) = value.as_str() {
        return Some(EntryValue::String(value.to_string()));
    }
    if let Some(value) = value.as_bool() {
        return Some(EntryValue::Bool(value));
    }
    if let Some(values) = value.as_array() {
        return values
            .iter()
            .map(|value| value.as_str().map(ToString::to_string))
            .collect::<Option<Vec<_>>>()
            .map(EntryValue::Strings);
    }
    value.as_inline_table().and_then(|values| {
        values
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|value| (key.to_string(), value.to_string()))
            })
            .collect::<Option<BTreeMap<_, _>>>()
            .map(EntryValue::StringMap)
    })
}

fn toml_add(
    raw: &str,
    top_level_key: &str,
    name: &str,
    entry: Vec<(String, EntryValue)>,
) -> Result<String, Error> {
    let mut document = parse_toml(raw)?;
    if document.get(top_level_key).is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        document.insert(top_level_key, Item::Table(table));
    }
    let parent = document
        .get_mut(top_level_key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| Error::Config {
            format: "TOML",
            message: format!("{top_level_key} must be a table"),
        })?;
    let mut server = Table::new();
    for (key, entry_value) in entry {
        server.insert(&key, toml_entry_value(entry_value));
    }
    parent.insert(name, Item::Table(server));
    Ok(document.to_string())
}

fn toml_remove(raw: &str, top_level_key: &str, name: &str) -> Result<String, Error> {
    if raw.trim().is_empty() {
        return Ok(raw.to_string());
    }
    let mut document = parse_toml(raw)?;
    let remove_parent = if let Some(parent) = document
        .get_mut(top_level_key)
        .and_then(Item::as_table_like_mut)
    {
        parent.remove(name);
        parent.is_empty()
    } else {
        false
    };
    if remove_parent {
        document.remove(top_level_key);
    }
    Ok(document.to_string())
}

fn toml_entry_value(entry: EntryValue) -> Item {
    match entry {
        EntryValue::String(value) => toml_value(value),
        EntryValue::Bool(value) => toml_value(value),
        EntryValue::Strings(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(value);
            }
            Item::Value(Value::Array(array))
        }
        EntryValue::StringMap(values) => {
            let mut table = InlineTable::new();
            for (key, value) in values {
                table.insert(&key, Value::from(value));
            }
            Item::Value(Value::InlineTable(table))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value as JsonValue;

    use super::Emitter;
    use crate::{AgentId, AgentScope, Error, McpServerSpec, resolve_agent_surface};

    fn emitter(agent: AgentId) -> Result<Emitter, Error> {
        Ok(Emitter::new(resolve_agent_surface(
            agent,
            AgentScope::System,
        )?))
    }

    fn stdio() -> McpServerSpec {
        McpServerSpec::Stdio {
            command: "gh-mcp".to_string(),
            args: vec!["serve".to_string()],
            env: BTreeMap::from([("KEY".to_string(), "value".to_string())]),
        }
    }

    fn http() -> McpServerSpec {
        McpServerSpec::Http {
            url: "https://example.com/mcp".to_string(),
            headers: BTreeMap::from([("Authorization".to_string(), "Bearer token".to_string())]),
        }
    }

    fn json_output(agent: AgentId, spec: &McpServerSpec) -> Result<JsonValue, Error> {
        let output = emitter(agent)?.add("", "gh", spec)?;
        serde_json::from_str(&output).map_err(|error| Error::Config {
            format: "test JSON",
            message: error.to_string(),
        })
    }

    #[test]
    fn catalog_shapes_emit_each_agent_quirk() -> Result<(), Error> {
        let vscode = json_output(AgentId::VsCode, &stdio())?;
        assert_eq!(vscode["servers"]["gh"]["type"], "stdio");

        let zed = json_output(AgentId::Zed, &stdio())?;
        assert_eq!(zed["context_servers"]["gh"]["source"], "custom");
        assert_eq!(zed["context_servers"]["gh"]["enabled"], true);

        let opencode = json_output(AgentId::OpenCode, &stdio())?;
        assert_eq!(opencode["mcp"]["servers"]["gh"]["command"][0], "gh-mcp");
        assert_eq!(opencode["mcp"]["servers"]["gh"]["command"][1], "serve");
        assert_eq!(
            opencode["mcp"]["servers"]["gh"]["environment"]["KEY"],
            "value"
        );
        assert_eq!(opencode["mcp"]["servers"]["gh"]["type"], "local");
        // OpenCode >= 2.0 has no `enabled` field; writing it would fail schema validation.
        assert!(opencode["mcp"]["servers"]["gh"].get("enabled").is_none());
        assert!(opencode["mcp"].get("gh").is_none());

        let opencode_http = json_output(AgentId::OpenCode, &http())?;
        assert_eq!(opencode_http["mcp"]["servers"]["gh"]["type"], "remote");
        assert!(
            opencode_http["mcp"]["servers"]["gh"]
                .get("enabled")
                .is_none()
        );

        let antigravity = json_output(AgentId::Antigravity, &http())?;
        assert_eq!(
            antigravity["mcpServers"]["gh"]["serverUrl"],
            "https://example.com/mcp"
        );
        assert!(antigravity["mcpServers"]["gh"].get("url").is_none());
        Ok(())
    }

    #[test]
    fn opencode_heals_a_legacy_top_level_entry() -> Result<(), Error> {
        // An older BrowserOS placed the server directly under `mcp` with an `enabled` field.
        // Re-adding must write it under `mcp.servers` without `enabled` and drop the stale
        // top-level entry so OpenCode >= 2.0 accepts the file.
        let emitter = emitter(AgentId::OpenCode)?;
        let raw = "{\n  \"mcp\": {\n    \"gh\": { \"type\": \"local\", \"command\": [\"old\"], \"enabled\": true }\n  }\n}\n";
        let added = emitter.add(raw, "gh", &stdio())?;
        let value: JsonValue = serde_json::from_str(&added).map_err(|error| Error::Config {
            format: "test JSON",
            message: error.to_string(),
        })?;
        assert_eq!(value["mcp"]["servers"]["gh"]["command"][0], "gh-mcp");
        assert_eq!(value["mcp"]["servers"]["gh"]["type"], "local");
        assert!(value["mcp"]["servers"]["gh"].get("enabled").is_none());
        // The stale top-level entry is gone.
        assert!(value["mcp"].get("gh").is_none());
        Ok(())
    }

    #[test]
    fn opencode_heal_preserves_a_disabled_legacy_entry() -> Result<(), Error> {
        // A user who disabled the old entry (`enabled: false`) must not be silently
        // re-enabled: the move carries the disable across as `disabled: true`.
        let emitter = emitter(AgentId::OpenCode)?;
        let raw = "{\n  \"mcp\": {\n    \"gh\": { \"type\": \"local\", \"command\": [\"old\"], \"enabled\": false }\n  }\n}\n";
        let added = emitter.add(raw, "gh", &stdio())?;
        let value: JsonValue = serde_json::from_str(&added).map_err(|error| Error::Config {
            format: "test JSON",
            message: error.to_string(),
        })?;
        assert_eq!(value["mcp"]["servers"]["gh"]["disabled"], true);
        assert!(value["mcp"]["servers"]["gh"].get("enabled").is_none());
        assert!(value["mcp"].get("gh").is_none());
        Ok(())
    }

    #[test]
    fn claude_code_tags_remote_transports_but_not_stdio() -> Result<(), Error> {
        let stdio = json_output(AgentId::ClaudeCode, &stdio())?;
        assert!(stdio["mcpServers"]["gh"].get("type").is_none());
        let http = json_output(AgentId::ClaudeCode, &http())?;
        assert_eq!(http["mcpServers"]["gh"]["type"], "http");
        let sse = json_output(
            AgentId::ClaudeCode,
            &McpServerSpec::Sse {
                url: "https://example.com/sse".to_string(),
                headers: BTreeMap::new(),
            },
        )?;
        assert_eq!(sse["mcpServers"]["gh"]["type"], "sse");
        Ok(())
    }

    #[test]
    fn jsonc_add_remove_preserves_comments_and_siblings_byte_for_byte() -> Result<(), Error> {
        let raw = "{\n  // root comment\n  \"theme\": \"dark\",\n  \"mcp\": {\n    // existing server\n    \"servers\": {\n      \"other\": { \"type\": \"local\", \"command\": [\"other\"] }\n    }\n  },\n  \"tail\": true\n}\n";
        let emitter = emitter(AgentId::OpenCode)?;
        let added = emitter.add(raw, "gh", &stdio())?;
        assert!(added.contains("// root comment"));
        assert!(added.contains("\"other\": { \"type\": \"local\", \"command\": [\"other\"] }"));
        assert!(added.contains("\"tail\": true"));
        let removed = emitter.remove(&added, "gh")?;
        assert_eq!(removed, raw);
        Ok(())
    }

    #[test]
    fn opencode_writes_under_mcp_servers_and_tolerates_a_utf8_bom() -> Result<(), Error> {
        let emitter = emitter(AgentId::OpenCode)?;
        let raw = "\u{feff}{\n  // keep\n  \"mcp\": {\n    \"servers\": {\n      \"other\": { \"type\": \"local\", \"command\": [\"other\"] }\n    }\n  }\n}\n";
        assert_eq!(emitter.read(raw)?, vec!["other".to_string()]);
        let added = emitter.add(raw, "gh", &http())?;
        assert!(added.starts_with('\u{feff}'));
        let root = jsonc_parser::cst::CstRootNode::parse(
            added.trim_start_matches('\u{feff}'),
            &jsonc_parser::ParseOptions::default(),
        )
        .map_err(|error| Error::Config {
            format: "test JSONC",
            message: error.to_string(),
        })?;
        let value: JsonValue = root
            .object_value()
            .and_then(|object| object.to_serde_value())
            .ok_or_else(|| Error::Config {
                format: "test JSONC",
                message: "config root is not an object".to_string(),
            })?;
        assert_eq!(
            value["mcp"]["servers"]["gh"]["url"],
            "https://example.com/mcp"
        );
        assert!(value["mcp"].get("gh").is_none());
        assert!(emitter.inspect(&added, "gh")?.is_some());
        let removed = emitter.remove(&added, "gh")?;
        assert_eq!(removed, raw);
        Ok(())
    }

    #[test]
    fn bom_only_config_seeds_for_the_requested_agent_shape() -> Result<(), Error> {
        let emitter = emitter(AgentId::OpenCode)?;
        let added = emitter.add("\u{feff}", "gh", &stdio())?;
        assert!(added.starts_with('\u{feff}'));
        let value: JsonValue =
            serde_json::from_str(added.trim_start_matches('\u{feff}')).map_err(|error| {
                Error::Config {
                    format: "test JSON",
                    message: error.to_string(),
                }
            })?;
        assert_eq!(value["mcp"]["servers"]["gh"]["command"][0], "gh-mcp");
        Ok(())
    }

    #[test]
    fn json_remove_keeps_an_empty_container_and_blank_remove_is_unchanged() -> Result<(), Error> {
        let emitter = emitter(AgentId::Cursor)?;
        let added = emitter.add("", "gh", &stdio())?;
        let removed = emitter.remove(&added, "gh")?;
        let value: JsonValue = serde_json::from_str(&removed).map_err(|error| Error::Config {
            format: "test JSON",
            message: error.to_string(),
        })?;
        assert!(
            value["mcpServers"]
                .as_object()
                .is_some_and(|map| map.is_empty())
        );
        assert_eq!(emitter.remove(" \n", "gh")?, " \n");
        Ok(())
    }

    #[test]
    fn toml_preserves_comments_and_removes_the_empty_parent_table() -> Result<(), Error> {
        let raw = "# model comment\nmodel = \"gpt-5\"\n\n# existing comment\n[mcp_servers.keep]\ncommand = \"keep\"\n";
        let emitter = emitter(AgentId::Codex)?;
        let added = emitter.add(raw, "gh", &http())?;
        assert!(added.contains("# model comment\nmodel = \"gpt-5\""));
        assert!(added.contains("# existing comment\n[mcp_servers.keep]"));
        assert!(added.contains("[mcp_servers.gh]"));
        assert!(added.contains("http_headers"));
        let restored = emitter.remove(&added, "gh")?;
        assert_eq!(restored, raw);

        let only = emitter.add("# root\n", "gh", &stdio())?;
        let empty = emitter.remove(&only, "gh")?;
        assert!(!empty.contains("mcp_servers"));
        assert!(empty.contains("# root"));
        Ok(())
    }

    #[test]
    fn toml_inline_table_add_remove_preserves_formatting_and_removes_empty_parent()
    -> Result<(), Error> {
        let raw = "# model\nmodel = \"gpt-5\"\nmcp_servers = { browseros = { url = \"https://old.example/mcp\" } } # inline\n# tail\n";
        let emitter = emitter(AgentId::Codex)?;
        let added = emitter.add(raw, "BrowserClaw", &http())?;
        assert!(added.contains("browseros = { url = \"https://old.example/mcp\" }"));
        assert!(added.contains("# inline\n# tail"));
        assert_eq!(emitter.remove(&added, "BrowserClaw")?, raw);

        let removed_only = emitter.remove(raw, "browseros")?;
        assert!(!removed_only.contains("mcp_servers"));
        assert!(removed_only.contains("# model\nmodel = \"gpt-5\""));
        assert!(removed_only.contains("# tail"));
        Ok(())
    }
}
