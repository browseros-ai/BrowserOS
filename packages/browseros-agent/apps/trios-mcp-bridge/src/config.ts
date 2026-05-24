/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge — Configuration
 */

export interface BridgeConfig {
  /** Port for the bridge MCP server (default: 9200) */
  port: number
  /** BrowserOS MCP server URL (default: http://127.0.0.1:9105/mcp) */
  browserosMcpUrl: string
  /** GitButler CLI path (default: "but") */
  gitbutlerCliPath: string
  /** Whether to use GitButler internal MCP tools (default: true) */
  gitbutlerInternal: boolean
  /** t27 CLI path (default: "tri") */
  triCliPath: string
  /** Working directory for git operations */
  workingDir: string
  /** Log level: "debug" | "info" | "warn" | "error" */
  logLevel: string
  /** trios-mcp-rag binary path (default: "trios-mcp-rag") */
  triosRagCliPath: string
  /** PostgreSQL DSN for trios-mcp-rag (default: null — uses RAG binary fallback or env) */
  databaseUrl: string | null
  /** Railway MCP server URL (default: null) */
  railwayMcpUrl: string | null
  /** trios-mcp-github binary path (default: "trios-mcp-github") */
  githubCliPath: string
}

const DEFAULT_CONFIG: BridgeConfig = {
  port: 9200,
  browserosMcpUrl: 'http://127.0.0.1:9105/mcp',
  gitbutlerCliPath: 'but',
  gitbutlerInternal: true,
  triCliPath: 'tri',
  workingDir: process.cwd(),
  logLevel: 'info',
  triosRagCliPath: 'trios-mcp-rag',
  databaseUrl: null,
  railwayMcpUrl: null,
  githubCliPath: 'trios-mcp-github',
}

export function loadConfig(overrides?: Partial<BridgeConfig>): BridgeConfig {
  return {
    ...DEFAULT_CONFIG,
    port: Number(process.env.TRIONS_BRIDGE_PORT) || DEFAULT_CONFIG.port,
    browserosMcpUrl:
      process.env.TRIONS_BROWSEROS_MCP_URL || DEFAULT_CONFIG.browserosMcpUrl,
    gitbutlerCliPath:
      process.env.TRIONS_GITBUTLER_CLI || DEFAULT_CONFIG.gitbutlerCliPath,
    gitbutlerInternal:
      process.env.TRIONS_GITBUTLER_INTERNAL === 'false'
        ? false
        : DEFAULT_CONFIG.gitbutlerInternal,
    triCliPath: process.env.TRIONS_TRI_CLI || DEFAULT_CONFIG.triCliPath,
    workingDir: process.env.TRIONS_WORKING_DIR || DEFAULT_CONFIG.workingDir,
    logLevel: process.env.TRIONS_LOG_LEVEL || DEFAULT_CONFIG.logLevel,
    triosRagCliPath:
      process.env.TRIONS_RAG_CLI || DEFAULT_CONFIG.triosRagCliPath,
    databaseUrl:
      process.env.DATABASE_URL || process.env.RAILWAY_SSOT_URL || null,
    railwayMcpUrl: process.env.RAILWAY_MCP_URL || DEFAULT_CONFIG.railwayMcpUrl,
    githubCliPath:
      process.env.TRIOS_GITHUB_CLI ||
      process.env.TRIONS_GITHUB_CLI ||
      DEFAULT_CONFIG.githubCliPath,
    ...overrides,
  }
}
