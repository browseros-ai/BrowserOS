// AgentId re-exports the 23-client union declared in the internal
// catalog. Kept as a name alias for API surface continuity: consumers
// import `AgentId` from the package root, the catalog's `ClientId`
// stays internal.
import type { ClientId } from './_catalog/types'

export type AgentId = ClientId

export type AgentScope = 'system' | 'project'

export type McpTransport = 'stdio' | 'sse' | 'http'

export interface AgentInfo {
  id: AgentId
  displayName: string
  /** Absolute path of the config file we would write to (or null when unresolvable on this OS). */
  configPath: string | null
  /** True iff one of the agent's `installCheckPaths` resolves on disk. */
  installed: boolean
}

export interface McpStdioSpec {
  transport: 'stdio'
  command: string
  args?: string[]
  env?: Record<string, string>
}

export interface McpSseSpec {
  transport: 'sse'
  url: string
  headers?: Record<string, string>
}

export interface McpHttpSpec {
  transport: 'http'
  url: string
  headers?: Record<string, string>
}

export type McpServerSpec = McpStdioSpec | McpSseSpec | McpHttpSpec

/**
 * The unit of work every mutating verb takes. Caller-owned data:
 * `name` is the manifest key; `spec` describes how the server is
 * invoked. Passed to `link()` directly; the library never persists
 * a server independently of a link.
 */
export interface McpServer {
  name: string
  spec: McpServerSpec
}

export interface ServerManifest {
  version: 1
  servers: Record<string, ManifestServerEntry>
}

export interface ManifestServerEntry {
  name: string
  spec: McpServerSpec
  addedAt: string
  links: Partial<Record<AgentId, ManifestLinkEntry>>
}

export interface ManifestLinkEntry {
  configPath: string
  createdAt: string
}
