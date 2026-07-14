// fallow-ignore-next-line unused-files
/**
 * Populated MCP client configuration catalog.
 *
 * 23 entries covering the full Smithery client roster. Every entry has:
 *   - `sources.firstParty`: URL to the client's own MCP documentation
 *     (primary source).
 *   - `sources.smithery`: URL to smithery-ai/cli src/config/clients.ts
 *     (corroborating cross-check; AGPL-3.0 design reference).
 *   - `sources.verified`: ISO date of last research/verification pass.
 *
 * Refresh cadence: the validator rejects any entry whose verified date
 * is more than 365 days old. When you touch an entry, bump its date.
 *
 * License: MIT. Interfaces and populated data authored by us. Smithery
 * is a design reference for the shape space and a corroborating source
 * for per-client values; no code copied verbatim. See
 * THIRD_PARTY_NOTICES.md.
 */

import type { ClientConfig } from './types'

const SMITHERY_URL =
  'https://github.com/smithery-ai/cli/blob/main/src/config/clients.ts'
const VERIFIED = '2026-07-06'

// -------------------------------------------------------------------
// Popular, well-documented clients
// -------------------------------------------------------------------

export const claudeDesktop: ClientConfig = {
  id: 'claude-desktop',
  displayName: 'Claude Desktop',
  installCheckPaths: {
    darwin: ['/Applications/Claude.app'],
    linux: ['$HOME/.config/claude'],
    win32: ['$APPDATA\\Claude'],
  },
  systemPaths: {
    darwin: [
      '$HOME/Library/Application Support/Claude/claude_desktop_config.json',
    ],
    linux: ['$HOME/.config/claude/claude_desktop_config.json'],
    win32: ['$APPDATA\\Claude\\claude_desktop_config.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://modelcontextprotocol.io/quickstart/user',
    smithery: SMITHERY_URL,
    notes:
      'Claude Desktop parses claude_desktop_config.json strictly. Entries without a `command` field are silently dropped on app launch. Hosted remote connectors are configured via the Claude Desktop UI, not this file.',
    verified: VERIFIED,
  },
}

export const claudeCode: ClientConfig = {
  id: 'claude-code',
  displayName: 'Claude Code',
  installCheckPaths: {
    darwin: ['$HOME/.claude'],
    linux: ['$HOME/.claude'],
    win32: ['$USERPROFILE\\.claude'],
  },
  systemPaths: {
    darwin: ['$CLAUDE_CONFIG_DIR/.claude.json', '$HOME/.claude.json'],
    linux: ['$CLAUDE_CONFIG_DIR/.claude.json', '$HOME/.claude.json'],
    win32: ['$CLAUDE_CONFIG_DIR\\.claude.json', '$USERPROFILE\\.claude.json'],
  },
  projectFile: '.mcp.json',
  format: 'json',
  supportedTransports: {
    system: ['stdio', 'sse', 'http'],
    project: ['stdio'],
  },
  stdio: { topLevelKey: 'mcpServers' },
  http: {
    tagKey: 'type',
    tagValue: 'http',
    sseTagValue: 'sse',
    supportsOAuth: true,
  },
  project: {
    stdio: { topLevelKey: 'mcpServers', tagKey: 'type', tagValue: 'stdio' },
  },
  sources: {
    firstParty: 'https://docs.claude.com/en/docs/claude-code/mcp',
    smithery: SMITHERY_URL,
    notes:
      'HTTP and SSE entries in ~/.claude.json require an explicit `type` field ("http", "sse", or "ws") or Claude Code emits a "url but no type" parse warning and skips the entry on launch. Stdio entries are accepted with or without a type tag. Project scope (.mcp.json) writes an explicit type: stdio tag per Claude Code project-scope docs.',
    verified: '2026-07-10',
  },
}

export const cursor: ClientConfig = {
  id: 'cursor',
  displayName: 'Cursor',
  installCheckPaths: {
    darwin: ['/Applications/Cursor.app'],
    linux: ['$HOME/.config/Cursor'],
    win32: ['$APPDATA\\Cursor'],
  },
  systemPaths: {
    darwin: ['$HOME/.cursor/mcp.json'],
    linux: ['$HOME/.cursor/mcp.json'],
    win32: ['$USERPROFILE\\.cursor\\mcp.json'],
  },
  projectFile: '.cursor/mcp.json',
  format: 'json',
  supportedTransports: {
    system: ['stdio', 'sse', 'http'],
    project: ['stdio', 'sse', 'http'],
  },
  stdio: { topLevelKey: 'mcpServers' },
  http: { tagKey: 'type', tagValue: 'http', supportsOAuth: true },
  project: {
    stdio: { topLevelKey: 'mcpServers' },
    http: { tagKey: 'type', tagValue: 'http', supportsOAuth: true },
  },
  sources: {
    firstParty: 'https://docs.cursor.com/context/model-context-protocol',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

const VSCODE_STDIO = {
  topLevelKey: 'servers',
  tagKey: 'type',
  tagValue: 'stdio',
} as const
const VSCODE_HTTP = {
  tagKey: 'type',
  tagValue: 'http',
  supportsOAuth: true,
} as const

export const vscode: ClientConfig = {
  id: 'vscode',
  displayName: 'Visual Studio Code',
  installCheckPaths: {
    darwin: ['/Applications/Visual Studio Code.app'],
    linux: ['$HOME/.config/Code'],
    win32: ['$APPDATA\\Code'],
  },
  systemPaths: {
    darwin: ['$HOME/Library/Application Support/Code/User/mcp.json'],
    linux: ['$HOME/.config/Code/User/mcp.json'],
    win32: ['$APPDATA\\Code\\User\\mcp.json'],
  },
  projectFile: '.vscode/mcp.json',
  format: 'json',
  supportedTransports: {
    system: ['stdio', 'sse', 'http'],
    project: ['stdio', 'sse', 'http'],
  },
  stdio: VSCODE_STDIO,
  http: VSCODE_HTTP,
  project: { stdio: VSCODE_STDIO, http: VSCODE_HTTP },
  sources: {
    firstParty: 'https://code.visualstudio.com/docs/copilot/chat/mcp-servers',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const vscodeInsiders: ClientConfig = {
  ...vscode,
  id: 'vscode-insiders',
  displayName: 'VS Code Insiders',
  installCheckPaths: {
    darwin: ['/Applications/Visual Studio Code - Insiders.app'],
    linux: ['$HOME/.config/Code - Insiders'],
    win32: ['$APPDATA\\Code - Insiders'],
  },
  systemPaths: {
    darwin: ['$HOME/Library/Application Support/Code - Insiders/User/mcp.json'],
    linux: ['$HOME/.config/Code - Insiders/User/mcp.json'],
    win32: ['$APPDATA\\Code - Insiders\\User\\mcp.json'],
  },
  sources: {
    firstParty: 'https://code.visualstudio.com/docs/copilot/chat/mcp-servers',
    smithery: SMITHERY_URL,
    notes:
      'Same shape as the stable VS Code channel; only the config directory differs.',
    verified: VERIFIED,
  },
}

export const gemini: ClientConfig = {
  id: 'gemini',
  displayName: 'Gemini CLI',
  installCheckPaths: {
    darwin: ['$HOME/.gemini'],
    linux: ['$HOME/.gemini'],
    win32: ['$USERPROFILE\\.gemini'],
  },
  systemPaths: {
    darwin: ['$HOME/.gemini/settings.json'],
    linux: ['$HOME/.gemini/settings.json'],
    win32: ['$USERPROFILE\\.gemini\\settings.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio', 'sse', 'http'] },
  stdio: { topLevelKey: 'mcpServers' },
  http: { tagKey: 'type', tagValue: 'http', supportsOAuth: true },
  sources: {
    firstParty:
      'https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const codex: ClientConfig = {
  id: 'codex',
  displayName: 'Codex',
  installCheckPaths: {
    darwin: ['$HOME/.codex'],
    linux: ['$HOME/.codex'],
    win32: ['$USERPROFILE\\.codex'],
  },
  systemPaths: {
    darwin: ['$HOME/.codex/config.toml'],
    linux: ['$HOME/.codex/config.toml'],
    win32: ['$USERPROFILE\\.codex\\config.toml'],
  },
  format: 'toml',
  supportedTransports: { system: ['stdio', 'http'] },
  stdio: { topLevelKey: 'mcp_servers' },
  http: { headerField: 'http_headers', supportsOAuth: true },
  sources: {
    firstParty: 'https://developers.openai.com/codex/mcp',
    smithery: SMITHERY_URL,
    notes:
      'TOML config. Streamable-HTTP entries carry `url`, optional `bearer_token_env_var`, `http_headers`, `env_http_headers`. SSE is not parsed.',
    verified: VERIFIED,
  },
}

export const zed: ClientConfig = {
  id: 'zed',
  displayName: 'Zed',
  installCheckPaths: {
    darwin: ['$HOME/.config/zed'],
    linux: ['$HOME/.config/zed'],
    win32: ['$APPDATA\\Zed'],
  },
  systemPaths: {
    darwin: ['$HOME/.config/zed/settings.json'],
    linux: ['$HOME/.config/zed/settings.json'],
    win32: ['$APPDATA\\Zed\\settings.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio', 'sse', 'http'] },
  stdio: {
    topLevelKey: 'context_servers',
    injects: { source: 'custom', enabled: true },
  },
  http: {
    injects: { source: 'custom', enabled: true },
    supportsOAuth: true,
  },
  sources: {
    firstParty: 'https://zed.dev/docs/ai/mcp',
    smithery: SMITHERY_URL,
    notes:
      'Zed calls them "context_servers". The block sits alongside other settings, so writes are non-destructive under that key.',
    verified: VERIFIED,
  },
}

// -------------------------------------------------------------------
// Additional clients from Smithery's full roster
// -------------------------------------------------------------------

export const cline: ClientConfig = {
  id: 'cline',
  displayName: 'Cline',
  installCheckPaths: {
    darwin: [
      '$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev',
    ],
    linux: ['$HOME/.config/Code/User/globalStorage/saoudrizwan.claude-dev'],
    win32: ['$APPDATA\\Code\\User\\globalStorage\\saoudrizwan.claude-dev'],
  },
  systemPaths: {
    darwin: [
      '$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json',
    ],
    linux: [
      '$HOME/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json',
    ],
    win32: [
      '$APPDATA\\Code\\User\\globalStorage\\saoudrizwan.claude-dev\\settings\\cline_mcp_settings.json',
    ],
  },
  format: 'json',
  supportedTransports: { system: ['stdio', 'http'] },
  stdio: { topLevelKey: 'mcpServers' },
  http: {
    tagKey: 'type',
    tagValue: 'streamableHttp',
    supportsOAuth: true,
  },
  sources: {
    firstParty: 'https://docs.cline.bot/mcp/mcp-server-development-protocol',
    smithery: SMITHERY_URL,
    notes:
      'Cline uses `type: "streamableHttp"` (camelCase) for remote HTTP, distinct from Kiro\'s `streamable-http`.',
    verified: VERIFIED,
  },
}

export const opencode: ClientConfig = {
  id: 'opencode',
  displayName: 'OpenCode',
  installCheckPaths: {
    darwin: [
      '$XDG_CONFIG_HOME/opencode',
      '$HOME/.config/opencode',
      '$HOME/.opencode',
    ],
    linux: [
      '$XDG_CONFIG_HOME/opencode',
      '$HOME/.config/opencode',
      '$HOME/.opencode',
    ],
    win32: ['$USERPROFILE\\.config\\opencode', '$USERPROFILE\\.opencode'],
  },
  systemPaths: {
    darwin: [
      '$XDG_CONFIG_HOME/opencode/opencode.json',
      '$HOME/.config/opencode/opencode.json',
      '$XDG_CONFIG_HOME/opencode/opencode.jsonc',
      '$HOME/.config/opencode/opencode.jsonc',
      '$HOME/.opencode/opencode.jsonc',
    ],
    linux: [
      '$XDG_CONFIG_HOME/opencode/opencode.json',
      '$HOME/.config/opencode/opencode.json',
      '$XDG_CONFIG_HOME/opencode/opencode.jsonc',
      '$HOME/.config/opencode/opencode.jsonc',
      '$HOME/.opencode/opencode.jsonc',
    ],
    win32: [
      '$USERPROFILE\\.config\\opencode\\opencode.json',
      '$USERPROFILE\\.config\\opencode\\opencode.jsonc',
      '$USERPROFILE\\.opencode\\opencode.jsonc',
    ],
  },
  format: 'jsonc',
  supportedTransports: { system: ['stdio', 'sse', 'http'] },
  stdio: {
    topLevelKey: 'mcp',
    envField: 'environment',
    commandAsArray: true,
    injects: { type: 'local', enabled: true },
  },
  http: {
    tagKey: 'type',
    tagValue: 'remote',
    injects: { enabled: true },
    supportsOAuth: true,
  },
  sources: {
    firstParty: 'https://opencode.ai/docs/mcp',
    smithery: SMITHERY_URL,
    notes:
      'Command written as a single array of [command, ...args] under `command`. `env` renamed to `environment`. Both stdio and remote entries carry a `type` field (`local` vs `remote`).',
    verified: VERIFIED,
  },
}

export const goose: ClientConfig = {
  id: 'goose',
  displayName: 'Goose',
  installCheckPaths: {
    darwin: ['$HOME/.config/goose'],
    linux: ['$HOME/.config/goose'],
    win32: ['$APPDATA\\Block\\goose'],
  },
  systemPaths: {
    darwin: ['$HOME/.config/goose/config.yaml'],
    linux: ['$HOME/.config/goose/config.yaml'],
    win32: ['$APPDATA\\Block\\goose\\config\\config.yaml'],
  },
  format: 'yaml',
  supportedTransports: { system: ['stdio', 'http'] },
  stdio: {
    topLevelKey: 'extensions',
    commandField: 'cmd',
    envField: 'envs',
    tagKey: 'type',
    tagValue: 'stdio',
    keyTransform: 'simpleName',
  },
  http: { tagKey: 'type', tagValue: 'http', supportsOAuth: true },
  sources: {
    firstParty:
      'https://block.github.io/goose/docs/getting-started/using-extensions',
    smithery: SMITHERY_URL,
    notes:
      "Extension keys are letters-only lowercase (Goose's `simpleName` transform). Command renamed to `cmd`, env renamed to `envs`. Docker's upstream config also hardcodes several static fields (`bundled: null`, `description`, `enabled: true`); we do not emit those unless a consumer asks.",
    verified: VERIFIED,
  },
}

export const kiro: ClientConfig = {
  id: 'kiro',
  displayName: 'Kiro',
  installCheckPaths: {
    darwin: ['$HOME/.kiro'],
    linux: ['$HOME/.kiro'],
    win32: ['$USERPROFILE\\.kiro'],
  },
  systemPaths: {
    darwin: ['$HOME/.kiro/settings/mcp.json'],
    linux: ['$HOME/.kiro/settings/mcp.json'],
    win32: ['$USERPROFILE\\.kiro\\settings\\mcp.json'],
  },
  projectFile: '.kiro/settings/mcp.json',
  format: 'json',
  supportedTransports: {
    system: ['stdio', 'http'],
    project: ['stdio', 'http'],
  },
  stdio: { topLevelKey: 'mcpServers' },
  http: {
    tagKey: 'type',
    tagValue: 'streamable-http',
    supportsOAuth: true,
  },
  project: {
    stdio: { topLevelKey: 'mcpServers' },
    http: {
      tagKey: 'type',
      tagValue: 'streamable-http',
      supportsOAuth: true,
    },
  },
  sources: {
    firstParty: 'https://kiro.dev/docs/mcp',
    smithery: SMITHERY_URL,
    notes:
      'Kiro uses `type: "streamable-http"` (kebab-case), the spec-canonical form.',
    verified: VERIFIED,
  },
}

export const windsurf: ClientConfig = {
  id: 'windsurf',
  displayName: 'Windsurf',
  installCheckPaths: {
    darwin: ['$HOME/.codeium/windsurf'],
    linux: ['$HOME/.codeium/windsurf'],
    win32: ['$USERPROFILE\\.codeium\\windsurf'],
  },
  systemPaths: {
    darwin: ['$HOME/.codeium/windsurf/mcp_config.json'],
    linux: ['$HOME/.codeium/windsurf/mcp_config.json'],
    win32: ['$USERPROFILE\\.codeium\\windsurf\\mcp_config.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio', 'http'] },
  stdio: { topLevelKey: 'mcpServers' },
  http: {
    urlField: 'serverUrl',
    supportsOAuth: true,
  },
  sources: {
    firstParty: 'https://docs.windsurf.com/windsurf/mcp',
    smithery: SMITHERY_URL,
    notes:
      'Windsurf calls the URL field `serverUrl` for remote entries, not the default `url`.',
    verified: VERIFIED,
  },
}

export const witsy: ClientConfig = {
  id: 'witsy',
  displayName: 'Witsy',
  installCheckPaths: {
    darwin: ['$HOME/Library/Application Support/Witsy'],
    linux: ['$HOME/.config/Witsy'],
    win32: ['$APPDATA\\Witsy'],
  },
  systemPaths: {
    darwin: ['$HOME/Library/Application Support/Witsy/settings.json'],
    linux: ['$HOME/.config/Witsy/settings.json'],
    win32: ['$APPDATA\\Witsy\\settings.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://github.com/nbonamy/witsy',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const enconvo: ClientConfig = {
  id: 'enconvo',
  displayName: 'Enconvo',
  installCheckPaths: {
    darwin: ['$HOME/.config/enconvo'],
    linux: ['$HOME/.config/enconvo'],
    win32: ['$USERPROFILE\\.config\\enconvo'],
  },
  systemPaths: {
    darwin: ['$HOME/.config/enconvo/mcp_config.json'],
    linux: ['$HOME/.config/enconvo/mcp_config.json'],
    win32: ['$USERPROFILE\\.config\\enconvo\\mcp_config.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://enconvo.com',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const roocode: ClientConfig = {
  id: 'roocode',
  displayName: 'Roo Code',
  installCheckPaths: {
    darwin: [
      '$HOME/Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline',
    ],
    linux: ['$HOME/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline'],
    win32: ['$APPDATA\\Code\\User\\globalStorage\\rooveterinaryinc.roo-cline'],
  },
  systemPaths: {
    darwin: [
      '$HOME/Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json',
    ],
    linux: [
      '$HOME/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json',
    ],
    win32: [
      '$APPDATA\\Code\\User\\globalStorage\\rooveterinaryinc.roo-cline\\settings\\mcp_settings.json',
    ],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://docs.roocode.com/features/mcp/overview',
    smithery: SMITHERY_URL,
    notes:
      "Roo Code is a VS Code extension (fork of Cline). MCP settings live in the extension's globalStorage folder.",
    verified: VERIFIED,
  },
}

export const boltai: ClientConfig = {
  id: 'boltai',
  displayName: 'BoltAI',
  installCheckPaths: {
    darwin: ['$HOME/.boltai'],
    linux: ['$HOME/.boltai'],
    win32: ['$USERPROFILE\\.boltai'],
  },
  systemPaths: {
    darwin: ['$HOME/.boltai/mcp.json'],
    linux: ['$HOME/.boltai/mcp.json'],
    win32: ['$USERPROFILE\\.boltai\\mcp.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://boltai.com',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const amazonBedrock: ClientConfig = {
  id: 'amazon-bedrock',
  displayName: 'Amazon Bedrock',
  installCheckPaths: {
    darwin: ['$HOME/Amazon Bedrock Client'],
    linux: ['$HOME/Amazon Bedrock Client'],
    win32: ['$USERPROFILE\\Amazon Bedrock Client'],
  },
  systemPaths: {
    darwin: ['$HOME/Amazon Bedrock Client/mcp_config.json'],
    linux: ['$HOME/Amazon Bedrock Client/mcp_config.json'],
    win32: ['$USERPROFILE\\Amazon Bedrock Client\\mcp_config.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://docs.aws.amazon.com/bedrock/latest/userguide/',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const amazonq: ClientConfig = {
  id: 'amazonq',
  displayName: 'Amazon Q',
  installCheckPaths: {
    darwin: ['$HOME/.aws/amazonq'],
    linux: ['$HOME/.aws/amazonq'],
    win32: ['$USERPROFILE\\.aws\\amazonq'],
  },
  systemPaths: {
    darwin: ['$HOME/.aws/amazonq/mcp.json'],
    linux: ['$HOME/.aws/amazonq/mcp.json'],
    win32: ['$USERPROFILE\\.aws\\amazonq\\mcp.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty:
      'https://docs.aws.amazon.com/amazonq/latest/qdeveloper-ug/command-line-mcp.html',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const tome: ClientConfig = {
  id: 'tome',
  displayName: 'Tome',
  installCheckPaths: {
    darwin: ['$HOME/.tome'],
    linux: ['$HOME/.tome'],
    win32: ['$USERPROFILE\\.tome'],
  },
  systemPaths: {
    darwin: ['$HOME/.tome/mcp_config.json'],
    linux: ['$HOME/.tome/mcp_config.json'],
    win32: ['$USERPROFILE\\.tome\\mcp_config.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio'] },
  stdio: { topLevelKey: 'mcpServers' },
  sources: {
    firstParty: 'https://github.com/runebookai/tome',
    smithery: SMITHERY_URL,
    verified: VERIFIED,
  },
}

export const librechat: ClientConfig = {
  id: 'librechat',
  displayName: 'LibreChat',
  installCheckPaths: {
    darwin: ['$HOME/LibreChat'],
    linux: ['$HOME/LibreChat'],
    win32: ['$USERPROFILE\\LibreChat'],
  },
  systemPaths: {
    darwin: [
      '$LIBRECHAT_CONFIG_DIR/LibreChat/librechat.yaml',
      '$HOME/LibreChat/librechat.yaml',
    ],
    linux: [
      '$LIBRECHAT_CONFIG_DIR/LibreChat/librechat.yaml',
      '$HOME/LibreChat/librechat.yaml',
    ],
    win32: [
      '$LIBRECHAT_CONFIG_DIR\\LibreChat\\librechat.yaml',
      '$USERPROFILE\\LibreChat\\librechat.yaml',
    ],
  },
  format: 'yaml',
  supportedTransports: { system: ['stdio', 'sse', 'http'] },
  stdio: { topLevelKey: 'mcpServers' },
  http: { tagKey: 'type', tagValue: 'http', supportsOAuth: true },
  sources: {
    firstParty:
      'https://www.librechat.ai/docs/configuration/librechat_yaml/object_structure/mcp_servers',
    smithery: SMITHERY_URL,
    notes:
      'YAML config file. mcpServers block accepts stdio (command/args) and remote (url) entries.',
    verified: VERIFIED,
  },
}

export const antigravity: ClientConfig = {
  id: 'antigravity',
  displayName: 'Antigravity',
  installCheckPaths: {
    darwin: ['$HOME/.gemini/antigravity'],
    linux: ['$HOME/.gemini/antigravity'],
    win32: ['$USERPROFILE\\.gemini\\antigravity'],
  },
  systemPaths: {
    darwin: ['$HOME/.gemini/antigravity/mcp_config.json'],
    linux: ['$HOME/.gemini/antigravity/mcp_config.json'],
    win32: ['$USERPROFILE\\.gemini\\antigravity\\mcp_config.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio', 'http'] },
  stdio: { topLevelKey: 'mcpServers' },
  http: {
    urlField: 'serverUrl',
    supportsOAuth: true,
  },
  sources: {
    firstParty: 'https://antigravity.google/',
    smithery: SMITHERY_URL,
    notes:
      "Google's Antigravity editor. Uses `serverUrl` for remote entries (matches Windsurf's convention).",
    verified: VERIFIED,
  },
}

export const trae: ClientConfig = {
  id: 'trae',
  displayName: 'Trae',
  installCheckPaths: {
    darwin: ['$HOME/Library/Application Support/Trae'],
    linux: ['$HOME/.config/Trae'],
    win32: ['$APPDATA\\Trae'],
  },
  systemPaths: {
    darwin: ['$HOME/Library/Application Support/Trae/User/mcp.json'],
    linux: ['$HOME/.config/Trae/User/mcp.json'],
    win32: ['$APPDATA\\Trae\\User\\mcp.json'],
  },
  format: 'json',
  supportedTransports: { system: ['stdio', 'http'] },
  stdio: { topLevelKey: 'mcpServers' },
  http: { tagKey: 'type', tagValue: 'http', supportsOAuth: true },
  sources: {
    firstParty: 'https://docs.trae.ai/ide/model-context-protocol',
    smithery: SMITHERY_URL,
    notes:
      'Trae is a VS Code fork with a dedicated mcp.json file under `<App Support>/Trae/User/`.',
    verified: VERIFIED,
  },
}

// -------------------------------------------------------------------
// Aggregated catalog
// -------------------------------------------------------------------

export const CATALOG: ReadonlyArray<ClientConfig> = [
  amazonBedrock,
  amazonq,
  antigravity,
  boltai,
  claudeCode,
  claudeDesktop,
  cline,
  codex,
  cursor,
  enconvo,
  gemini,
  goose,
  kiro,
  librechat,
  opencode,
  roocode,
  tome,
  trae,
  vscode,
  vscodeInsiders,
  windsurf,
  witsy,
  zed,
]

type CatalogById = { readonly [K in ClientConfig['id']]: ClientConfig }

export const CATALOG_BY_ID: CatalogById = CATALOG.reduce(
  (acc, entry) => Object.assign(acc, { [entry.id]: entry }),
  Object.create(null) as CatalogById,
)
