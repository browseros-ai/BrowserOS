use std::{io, path::PathBuf};

use thiserror::Error;

use crate::{AgentId, McpTransport};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Agent not supported: {agent}")]
    AgentNotSupported { agent: String },

    #[error(
        "Entry \"{server_name}\" in {} was not written by agent-mcp-manager for agent \"{agent}\". Refusing to remove.",
        config_path.display()
    )]
    ForeignEntry {
        server_name: String,
        agent: AgentId,
        config_path: PathBuf,
    },

    #[error(
        "Agent \"{agent}\" does not support the \"{transport}\" transport (supported: {}). {hint}",
        supported.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    UnsupportedTransport {
        agent: AgentId,
        transport: McpTransport,
        supported: Vec<McpTransport>,
        hint: String,
    },

    #[error("Cannot resolve config path for agent \"{agent}\": {reason}")]
    UnresolvedConfigPath { agent: AgentId, reason: String },

    #[error("Invalid MCP server spec: {reason}")]
    InvalidServerSpec { reason: String },

    #[error(
        "Agent \"{agent}\" does not appear to be installed on this machine. The library needs \"{}\" or its parent directory \"{}\" to exist before it can write an MCP entry. Install {agent} and launch it at least once, or pass an explicit \"configPath\" to write to a custom location.",
        config_path.display(),
        parent_dir.display()
    )]
    AgentNotInstalled {
        agent: AgentId,
        config_path: PathBuf,
        parent_dir: PathBuf,
    },

    #[error("{message}")]
    Manifest { message: String },

    #[error("Invalid {format} agent config: {message}")]
    Config {
        format: &'static str,
        message: String,
    },

    #[error("Failed to {operation} {}: {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl Error {
    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}
