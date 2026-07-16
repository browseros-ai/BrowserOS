use std::path::PathBuf;

use crate::{
    AgentId, AgentInfo, AgentScope, Error, McpTransport,
    paths::{any_exists, pick_config_path, selected_os_paths},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerOsPaths {
    pub darwin: &'static [&'static str],
    pub linux: &'static [&'static str],
    pub windows: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Jsonc,
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTransform {
    SimpleName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectValue {
    String(&'static str),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdioShape {
    pub top_level_key: &'static str,
    pub command_field: Option<&'static str>,
    pub args_field: Option<&'static str>,
    pub env_field: Option<&'static str>,
    pub command_as_array: bool,
    pub tag_key: Option<&'static str>,
    pub tag_value: Option<&'static str>,
    pub injects: &'static [(&'static str, InjectValue)],
    pub key_transform: Option<KeyTransform>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpShape {
    pub url_field: Option<&'static str>,
    pub header_field: Option<&'static str>,
    pub tag_key: Option<&'static str>,
    pub tag_value: Option<&'static str>,
    pub sse_tag_value: Option<&'static str>,
    pub injects: &'static [(&'static str, InjectValue)],
    pub supports_oauth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSurface {
    pub stdio: StdioShape,
    pub http: Option<HttpShape>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientConfigSources {
    pub first_party: &'static str,
    pub smithery: Option<&'static str>,
    pub notes: Option<&'static str>,
    pub verified: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientConfig {
    pub id: AgentId,
    pub display_name: &'static str,
    pub install_check_paths: PerOsPaths,
    pub system_paths: PerOsPaths,
    pub project_file: Option<&'static str>,
    pub format: ConfigFormat,
    pub system_transports: &'static [McpTransport],
    pub project_transports: Option<&'static [McpTransport]>,
    pub stdio: StdioShape,
    pub http: Option<HttpShape>,
    pub project: Option<ProjectSurface>,
    pub sources: ClientConfigSources,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentSurface {
    pub client: &'static ClientConfig,
    pub supported_transports: &'static [McpTransport],
    pub stdio: StdioShape,
    pub http: Option<HttpShape>,
}

const SMITHERY_URL: &str = "https://github.com/smithery-ai/cli/blob/main/src/config/clients.ts";
const VERIFIED: &str = "2026-07-06";
const NONE: &[(&str, InjectValue)] = &[];
const ZED_INJECTS: &[(&str, InjectValue)] = &[
    ("source", InjectValue::String("custom")),
    ("enabled", InjectValue::Bool(true)),
];
const OPENCODE_STDIO_INJECTS: &[(&str, InjectValue)] = &[
    ("type", InjectValue::String("local")),
    ("enabled", InjectValue::Bool(true)),
];
const OPENCODE_HTTP_INJECTS: &[(&str, InjectValue)] = &[("enabled", InjectValue::Bool(true))];

const STDIO_SSE_HTTP: &[McpTransport] =
    &[McpTransport::Stdio, McpTransport::Sse, McpTransport::Http];
const STDIO_HTTP: &[McpTransport] = &[McpTransport::Stdio, McpTransport::Http];
const STDIO_ONLY: &[McpTransport] = &[McpTransport::Stdio];

const fn paths(
    darwin: &'static [&'static str],
    linux: &'static [&'static str],
    windows: &'static [&'static str],
) -> PerOsPaths {
    PerOsPaths {
        darwin,
        linux,
        windows,
    }
}

const fn stdio(top_level_key: &'static str) -> StdioShape {
    StdioShape {
        top_level_key,
        command_field: None,
        args_field: None,
        env_field: None,
        command_as_array: false,
        tag_key: None,
        tag_value: None,
        injects: NONE,
        key_transform: None,
    }
}

const CLAUDE_CODE: ClientConfig = ClientConfig {
    id: AgentId::ClaudeCode,
    display_name: "Claude Code",
    install_check_paths: paths(
        &["$HOME/.claude"],
        &["$HOME/.claude"],
        &["$USERPROFILE\\.claude"],
    ),
    system_paths: paths(
        &["$CLAUDE_CONFIG_DIR/.claude.json", "$HOME/.claude.json"],
        &["$CLAUDE_CONFIG_DIR/.claude.json", "$HOME/.claude.json"],
        &[
            "$CLAUDE_CONFIG_DIR\\.claude.json",
            "$USERPROFILE\\.claude.json",
        ],
    ),
    project_file: Some(".mcp.json"),
    format: ConfigFormat::Json,
    system_transports: STDIO_SSE_HTTP,
    project_transports: Some(STDIO_ONLY),
    stdio: stdio("mcpServers"),
    http: Some(HttpShape {
        url_field: None,
        header_field: None,
        tag_key: Some("type"),
        tag_value: Some("http"),
        sse_tag_value: Some("sse"),
        injects: NONE,
        supports_oauth: true,
    }),
    project: Some(ProjectSurface {
        stdio: StdioShape {
            tag_key: Some("type"),
            tag_value: Some("stdio"),
            ..stdio("mcpServers")
        },
        http: None,
    }),
    sources: ClientConfigSources {
        first_party: "https://docs.claude.com/en/docs/claude-code/mcp",
        smithery: Some(SMITHERY_URL),
        notes: Some(
            "HTTP and SSE entries in ~/.claude.json require an explicit `type` field (\"http\", \"sse\", or \"ws\") or Claude Code emits a \"url but no type\" parse warning and skips the entry on launch. Stdio entries are accepted with or without a type tag. Project scope (.mcp.json) writes an explicit type: stdio tag per Claude Code project-scope docs.",
        ),
        verified: "2026-07-10",
    },
};

const CURSOR: ClientConfig = ClientConfig {
    id: AgentId::Cursor,
    display_name: "Cursor",
    install_check_paths: paths(
        &["/Applications/Cursor.app"],
        &["$HOME/.config/Cursor"],
        &["$APPDATA\\Cursor"],
    ),
    system_paths: paths(
        &["$HOME/.cursor/mcp.json"],
        &["$HOME/.cursor/mcp.json"],
        &["$USERPROFILE\\.cursor\\mcp.json"],
    ),
    project_file: Some(".cursor/mcp.json"),
    format: ConfigFormat::Json,
    system_transports: STDIO_SSE_HTTP,
    project_transports: Some(STDIO_SSE_HTTP),
    stdio: stdio("mcpServers"),
    http: Some(HttpShape {
        url_field: None,
        header_field: None,
        tag_key: Some("type"),
        tag_value: Some("http"),
        sse_tag_value: None,
        injects: NONE,
        supports_oauth: true,
    }),
    project: Some(ProjectSurface {
        stdio: stdio("mcpServers"),
        http: Some(HttpShape {
            url_field: None,
            header_field: None,
            tag_key: Some("type"),
            tag_value: Some("http"),
            sse_tag_value: None,
            injects: NONE,
            supports_oauth: true,
        }),
    }),
    sources: ClientConfigSources {
        first_party: "https://docs.cursor.com/context/model-context-protocol",
        smithery: Some(SMITHERY_URL),
        notes: None,
        verified: VERIFIED,
    },
};

const VSCODE_STDIO: StdioShape = StdioShape {
    tag_key: Some("type"),
    tag_value: Some("stdio"),
    ..stdio("servers")
};
const VSCODE_HTTP: HttpShape = HttpShape {
    url_field: None,
    header_field: None,
    tag_key: Some("type"),
    tag_value: Some("http"),
    sse_tag_value: None,
    injects: NONE,
    supports_oauth: true,
};
const VSCODE: ClientConfig = ClientConfig {
    id: AgentId::VsCode,
    display_name: "Visual Studio Code",
    install_check_paths: paths(
        &["/Applications/Visual Studio Code.app"],
        &["$HOME/.config/Code"],
        &["$APPDATA\\Code"],
    ),
    system_paths: paths(
        &["$HOME/Library/Application Support/Code/User/mcp.json"],
        &["$HOME/.config/Code/User/mcp.json"],
        &["$APPDATA\\Code\\User\\mcp.json"],
    ),
    project_file: Some(".vscode/mcp.json"),
    format: ConfigFormat::Json,
    system_transports: STDIO_SSE_HTTP,
    project_transports: Some(STDIO_SSE_HTTP),
    stdio: VSCODE_STDIO,
    http: Some(VSCODE_HTTP),
    project: Some(ProjectSurface {
        stdio: VSCODE_STDIO,
        http: Some(VSCODE_HTTP),
    }),
    sources: ClientConfigSources {
        first_party: "https://code.visualstudio.com/docs/copilot/chat/mcp-servers",
        smithery: Some(SMITHERY_URL),
        notes: None,
        verified: VERIFIED,
    },
};

const CODEX: ClientConfig = ClientConfig {
    id: AgentId::Codex,
    display_name: "Codex",
    install_check_paths: paths(
        &["$HOME/.codex"],
        &["$HOME/.codex"],
        &["$USERPROFILE\\.codex"],
    ),
    system_paths: paths(
        &["$HOME/.codex/config.toml"],
        &["$HOME/.codex/config.toml"],
        &["$USERPROFILE\\.codex\\config.toml"],
    ),
    project_file: None,
    format: ConfigFormat::Toml,
    system_transports: STDIO_HTTP,
    project_transports: None,
    stdio: stdio("mcp_servers"),
    http: Some(HttpShape {
        url_field: None,
        header_field: Some("http_headers"),
        tag_key: None,
        tag_value: None,
        sse_tag_value: None,
        injects: NONE,
        supports_oauth: true,
    }),
    project: None,
    sources: ClientConfigSources {
        first_party: "https://developers.openai.com/codex/mcp",
        smithery: Some(SMITHERY_URL),
        notes: Some(
            "TOML config. Streamable-HTTP entries carry `url`, optional `bearer_token_env_var`, `http_headers`, `env_http_headers`. SSE is not parsed.",
        ),
        verified: VERIFIED,
    },
};

const ZED: ClientConfig = ClientConfig {
    id: AgentId::Zed,
    display_name: "Zed",
    install_check_paths: paths(
        &["$HOME/.config/zed"],
        &["$HOME/.config/zed"],
        &["$APPDATA\\Zed"],
    ),
    system_paths: paths(
        &["$HOME/.config/zed/settings.json"],
        &["$HOME/.config/zed/settings.json"],
        &["$APPDATA\\Zed\\settings.json"],
    ),
    project_file: None,
    format: ConfigFormat::Json,
    system_transports: STDIO_SSE_HTTP,
    project_transports: None,
    stdio: StdioShape {
        injects: ZED_INJECTS,
        ..stdio("context_servers")
    },
    http: Some(HttpShape {
        url_field: None,
        header_field: None,
        tag_key: None,
        tag_value: None,
        sse_tag_value: None,
        injects: ZED_INJECTS,
        supports_oauth: true,
    }),
    project: None,
    sources: ClientConfigSources {
        first_party: "https://zed.dev/docs/ai/mcp",
        smithery: Some(SMITHERY_URL),
        notes: Some(
            "Zed calls them \"context_servers\". The block sits alongside other settings, so writes are non-destructive under that key.",
        ),
        verified: VERIFIED,
    },
};

const OPENCODE: ClientConfig = ClientConfig {
    id: AgentId::OpenCode,
    display_name: "OpenCode",
    install_check_paths: paths(
        &[
            "$XDG_CONFIG_HOME/opencode",
            "$HOME/.config/opencode",
            "$HOME/.opencode",
        ],
        &[
            "$XDG_CONFIG_HOME/opencode",
            "$HOME/.config/opencode",
            "$HOME/.opencode",
        ],
        &["$USERPROFILE\\.config\\opencode", "$USERPROFILE\\.opencode"],
    ),
    system_paths: paths(
        &[
            "$XDG_CONFIG_HOME/opencode/opencode.json",
            "$HOME/.config/opencode/opencode.json",
            "$XDG_CONFIG_HOME/opencode/opencode.jsonc",
            "$HOME/.config/opencode/opencode.jsonc",
            "$HOME/.opencode/opencode.jsonc",
        ],
        &[
            "$XDG_CONFIG_HOME/opencode/opencode.json",
            "$HOME/.config/opencode/opencode.json",
            "$XDG_CONFIG_HOME/opencode/opencode.jsonc",
            "$HOME/.config/opencode/opencode.jsonc",
            "$HOME/.opencode/opencode.jsonc",
        ],
        &[
            "$USERPROFILE\\.config\\opencode\\opencode.json",
            "$USERPROFILE\\.config\\opencode\\opencode.jsonc",
            "$USERPROFILE\\.opencode\\opencode.jsonc",
        ],
    ),
    project_file: None,
    format: ConfigFormat::Jsonc,
    system_transports: STDIO_SSE_HTTP,
    project_transports: None,
    stdio: StdioShape {
        env_field: Some("environment"),
        command_as_array: true,
        injects: OPENCODE_STDIO_INJECTS,
        ..stdio("mcp")
    },
    http: Some(HttpShape {
        url_field: None,
        header_field: None,
        tag_key: Some("type"),
        tag_value: Some("remote"),
        sse_tag_value: None,
        injects: OPENCODE_HTTP_INJECTS,
        supports_oauth: true,
    }),
    project: None,
    sources: ClientConfigSources {
        first_party: "https://opencode.ai/docs/mcp",
        smithery: Some(SMITHERY_URL),
        notes: Some(
            "Command written as a single array of [command, ...args] under `command`. `env` renamed to `environment`. Both stdio and remote entries carry a `type` field (`local` vs `remote`).",
        ),
        verified: VERIFIED,
    },
};

const ANTIGRAVITY: ClientConfig = ClientConfig {
    id: AgentId::Antigravity,
    display_name: "Antigravity",
    install_check_paths: paths(
        &["$HOME/.gemini/antigravity"],
        &["$HOME/.gemini/antigravity"],
        &["$USERPROFILE\\.gemini\\antigravity"],
    ),
    system_paths: paths(
        &["$HOME/.gemini/antigravity/mcp_config.json"],
        &["$HOME/.gemini/antigravity/mcp_config.json"],
        &["$USERPROFILE\\.gemini\\antigravity\\mcp_config.json"],
    ),
    project_file: None,
    format: ConfigFormat::Json,
    system_transports: STDIO_HTTP,
    project_transports: None,
    stdio: stdio("mcpServers"),
    http: Some(HttpShape {
        url_field: Some("serverUrl"),
        header_field: None,
        tag_key: None,
        tag_value: None,
        sse_tag_value: None,
        injects: NONE,
        supports_oauth: true,
    }),
    project: None,
    sources: ClientConfigSources {
        first_party: "https://antigravity.google/",
        smithery: Some(SMITHERY_URL),
        notes: Some(
            "Google's Antigravity editor. Uses `serverUrl` for remote entries (matches Windsurf's convention).",
        ),
        verified: VERIFIED,
    },
};

const CATALOG: [&ClientConfig; 7] = [
    &CLAUDE_CODE,
    &CODEX,
    &CURSOR,
    &OPENCODE,
    &ANTIGRAVITY,
    &VSCODE,
    &ZED,
];

/// Returns the complete BrowserClaw harness catalog in stable order.
pub fn list_supported_agents() -> Vec<AgentId> {
    CATALOG.iter().map(|entry| entry.id).collect()
}

/// Checks whether a string is one of the seven supported agent identifiers.
pub fn is_agent_supported(agent: &str) -> bool {
    CATALOG.iter().any(|entry| entry.id.as_str() == agent)
}

pub(crate) fn get_catalog_entry(agent: AgentId) -> &'static ClientConfig {
    match agent {
        AgentId::ClaudeCode => &CLAUDE_CODE,
        AgentId::Codex => &CODEX,
        AgentId::Cursor => &CURSOR,
        AgentId::OpenCode => &OPENCODE,
        AgentId::Antigravity => &ANTIGRAVITY,
        AgentId::VsCode => &VSCODE,
        AgentId::Zed => &ZED,
    }
}

/// Resolves the active configuration shape and transport set for an agent.
pub fn resolve_agent_surface(agent: AgentId, scope: AgentScope) -> Result<AgentSurface, Error> {
    ensure_system_scope(agent, scope)?;
    let client = get_catalog_entry(agent);
    Ok(AgentSurface {
        client,
        supported_transports: client.system_transports,
        stdio: client.stdio,
        http: client.http,
    })
}

/// Resolves the first existing system config candidate, or the first resolvable candidate.
pub fn resolve_agent_mcp_config_path(agent: AgentId, scope: AgentScope) -> Result<PathBuf, Error> {
    ensure_system_scope(agent, scope)?;
    let candidates = selected_os_paths(&get_catalog_entry(agent).system_paths);
    if candidates.is_empty() {
        return Err(Error::UnresolvedConfigPath {
            agent,
            reason: format!(
                "no system config path configured for OS {}",
                std::env::consts::OS
            ),
        });
    }
    pick_config_path(candidates)?.ok_or_else(|| Error::UnresolvedConfigPath {
        agent,
        reason: "no system config path resolves (env vars unset?)".to_string(),
    })
}

/// Reports catalog install checks separately from config-path writability.
pub fn detect_installed_agents() -> Result<Vec<AgentInfo>, Error> {
    CATALOG
        .iter()
        .map(|entry| {
            let checks = selected_os_paths(&entry.install_check_paths);
            let installed = any_exists(checks)?;
            let config_path = resolve_agent_mcp_config_path(entry.id, AgentScope::System).ok();
            Ok(AgentInfo {
                id: entry.id,
                display_name: entry.display_name.to_string(),
                config_path,
                installed,
            })
        })
        .collect()
}

pub(crate) fn ensure_system_scope(agent: AgentId, scope: AgentScope) -> Result<(), Error> {
    if scope == AgentScope::System {
        return Ok(());
    }
    Err(Error::UnresolvedConfigPath {
        agent,
        reason: "project scope is not supported; only system scope is implemented".to_string(),
    })
}
