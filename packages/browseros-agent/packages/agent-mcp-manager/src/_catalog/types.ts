// fallow-ignore-next-line unused-files
/**
 * Hand-authored MCP client configuration catalog.
 *
 * Every published client is one `ClientConfig` value. Data lives in
 * `client-configs.ts`; interpretation lives in the emitters. Sources
 * were researched from each client's first-party MCP docs, cross-
 * checked against smithery-ai/cli (AGPL-3.0, design reference only).
 * See `THIRD_PARTY_NOTICES.md` for the Smithery disclosure.
 *
 * Schema pattern (interfaces + shape space) is informed by Smithery's
 * `src/config/clients.ts`. Interfaces and populated data are authored
 * by us under MIT; no source code from Smithery is incorporated.
 */

import type { McpTransport } from '../types'

/** Union of every client id we ship a config for. */
export type ClientId =
  | 'claude-desktop'
  | 'claude-code'
  | 'cursor'
  | 'vscode'
  | 'vscode-insiders'
  | 'gemini'
  | 'codex'
  | 'zed'
  | 'cline'
  | 'opencode'
  | 'goose'
  | 'kiro'
  | 'windsurf'
  | 'witsy'
  | 'enconvo'
  | 'roocode'
  | 'boltai'
  | 'amazon-bedrock'
  | 'amazonq'
  | 'tome'
  | 'librechat'
  | 'antigravity'
  | 'trae'

export interface PerOsPaths {
  darwin?: string[]
  linux?: string[]
  win32?: string[]
}

export type ConfigFormat = 'json' | 'jsonc' | 'yaml' | 'toml'

export interface StdioShape {
  /** Object key that holds the entry map. Examples: 'mcpServers', 'servers', 'mcp', 'context_servers', 'extensions'. */
  topLevelKey: string
  /** Field name for the executable command. Default 'command'. Goose uses 'cmd'. */
  commandField?: string
  /** Field name for the args array. Default 'args'. */
  argsField?: string
  /** Field name for the env-var map. Default 'env'. Goose uses 'envs'. OpenCode uses 'environment'. */
  envField?: string
  /**
   * When true, emit command + args as a single array under the command
   * field (e.g. `command: [<command>, ...<args>]`). OpenCode does this.
   */
  commandAsArray?: boolean
  /** When set, write `{ [tagKey]: tagValue }` alongside the base shape. */
  tagKey?: 'type' | 'transport'
  /** Value written under `tagKey`. Common: 'stdio' or 'local'. */
  tagValue?: string
  /** Static fields merged into every stdio entry (e.g. Zed's `source: 'custom', enabled: true`). */
  injects?: Record<string, unknown>
  /**
   * When set, transform the caller-supplied server name before using it
   * as the entry key. `'simpleName'` matches Docker/Goose semantics
   * (lowercase, letters only: `MCP_DOCKER` -> `mcpdocker`).
   */
  keyTransform?: 'simpleName'
}

export interface HttpShape {
  /** Field name for the URL. Default 'url'. Windsurf uses 'serverUrl'. */
  urlField?: string
  /** Field name for the header map. Default 'headers'. */
  headerField?: string
  /** When set, write `{ [tagKey]: tagValue }` for http entries. */
  tagKey?: 'type' | 'transport'
  /**
   * Value written under `tagKey` for http entries. Known values:
   * 'http', 'streamableHttp' (Cline), 'streamable-http' (Kiro),
   * 'remote' (OpenCode). Applied to both `http` and `sse` transports
   * unless `sseTagValue` is set.
   */
  tagValue?: string
  /**
   * Value written under `tagKey` for SSE entries specifically. Use when
   * the client accepts both http and sse and distinguishes them by tag
   * value: Claude Code writes `type: "http"` for http and `type: "sse"`
   * for sse. When unset, SSE entries use `tagValue` (the shared value).
   */
  sseTagValue?: string
  /** Static fields merged into every http entry. */
  injects?: Record<string, unknown>
  /**
   * Whether the client advertises OAuth support. Recorded for future
   * awareness; v0.0.4 does not act on it. Consumers thread bearer tokens
   * via `spec.headers` today.
   */
  supportsOAuth?: boolean
}

export interface ClientConfigSources {
  /** Required. URL to the client's first-party MCP documentation. */
  firstParty: string
  /** URL to the corroborating source we cross-checked at authoring time. Set when Smithery has an entry. */
  smithery?: string
  /** Free-form notes about drift between sources or edge cases. */
  notes?: string
  /** ISO date the entry was last verified against sources. Enforced by the validator (must be within 12 months). */
  verified: string
}

export interface ClientConfig {
  id: ClientId
  displayName: string
  installCheckPaths: PerOsPaths
  systemPaths: PerOsPaths
  /** Relative to projectRoot when scope is 'project'. */
  projectFile?: string
  format: ConfigFormat
  supportedTransports: {
    system: ReadonlyArray<McpTransport>
    /** Set when project scope diverges from system scope. */
    project?: ReadonlyArray<McpTransport>
  }
  stdio: StdioShape
  /** Present iff `supportedTransports.system` includes 'http' or 'sse'. Enforced by the validator. */
  http?: HttpShape
  /** Present when project scope has divergent write shapes. Enforced by the validator. */
  project?: {
    stdio: StdioShape
    http?: HttpShape
  }
  sources: ClientConfigSources
}
