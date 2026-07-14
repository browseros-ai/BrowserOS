/**
 * agent-mcp-manager public API (v0.0.4).
 *
 * The v0.0.3 class API (`createMcpManager`, `McpManager`) has been
 * removed. Consumers migrate to the functional verbs exported here or
 * to the low-level plan/apply primitives at
 * `agent-mcp-manager/lowlevel` for dry-run control.
 *
 * See README.md's Migration section for a verb-by-verb translation
 * table.
 */

export {
  detectInstalledAgents,
  getCatalogEntry,
  isAgentSupported,
  listSupportedAgents,
  resolveAgentMcpConfigPath,
  resolveAgentSurface,
} from './agents'
export type {
  BoundApi,
  DisconnectInputAPI,
  IsInstalledInput,
  IsInstalledResult,
  LinkInputAPI,
  ListedLink,
  ListLinksInputAPI,
  RemoveInputAPI,
  RescanInputAPI,
  UnlinkInputAPI,
} from './api'
export {
  bind,
  disconnect,
  isInstalled,
  link,
  list,
  listLinks,
  remove,
  rescan,
  unlink,
} from './api'

export {
  AgentNotInstalledError,
  AgentNotSupportedError,
  ForeignEntryError,
  InvalidServerSpecError,
  McpManagerError,
  ServerNotFoundError,
  UnresolvedConfigPathError,
  UnsupportedTransportError,
} from './errors'

export type {
  DisconnectPlanSummary,
  LinkPlanSummary,
  RemovePlanSummary,
  RescanReport,
  UnlinkPlanSummary,
} from './planner/types'

export type {
  AgentId,
  AgentInfo,
  AgentScope,
  ManifestLinkEntry,
  ManifestServerEntry,
  McpHttpSpec,
  McpServer,
  McpServerSpec,
  McpSseSpec,
  McpStdioSpec,
  McpTransport,
  ServerManifest,
} from './types'

export const VERSION = '0.0.4-rc.4'
