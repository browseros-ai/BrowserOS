import type { AgentId, McpTransport } from './types'

export class McpManagerError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message)
    this.name = 'McpManagerError'
    if (options?.cause !== undefined) {
      ;(this as { cause?: unknown }).cause = options.cause
    }
  }
}

export class AgentNotSupportedError extends McpManagerError {
  readonly agent: string
  constructor(agent: string) {
    super(`Agent not supported: ${agent}`)
    this.name = 'AgentNotSupportedError'
    this.agent = agent
  }
}

export class ServerNotFoundError extends McpManagerError {
  readonly serverName: string
  constructor(serverName: string) {
    super(`No server named "${serverName}" in the manifest`)
    this.name = 'ServerNotFoundError'
    this.serverName = serverName
  }
}

/**
 * Raised by `unlink()` when the on-disk entry under that key was not
 * written by this library (no manifest record). Callers can retry with
 * `force: true` (future) or run `rescan({ mode: 'merge' })` to adopt it.
 */
export class ForeignEntryError extends McpManagerError {
  readonly serverName: string
  readonly agent: AgentId
  readonly configPath: string
  constructor(serverName: string, agent: AgentId, configPath: string) {
    super(
      `Entry "${serverName}" in ${configPath} was not written by agent-mcp-manager for agent "${agent}". Refusing to remove.`,
    )
    this.name = 'ForeignEntryError'
    this.serverName = serverName
    this.agent = agent
    this.configPath = configPath
  }
}

export class InvalidServerSpecError extends McpManagerError {
  constructor(reason: string) {
    super(`Invalid MCP server spec: ${reason}`)
    this.name = 'InvalidServerSpecError'
  }
}

/**
 * Raised by `link()` when the requested transport is not one this agent's
 * config file actually accepts. The most common case is passing
 * `transport: 'http'` (or `'sse'`) for `claude-desktop`, whose parser
 * only validates stdio-shaped entries on disk; codex now accepts http
 * directly but still rejects sse. The `hint` field names the
 * `mcp-remote` wrapper pattern so callers can produce a stdio-shaped
 * spec the agent will accept.
 */
export class UnsupportedTransportError extends McpManagerError {
  readonly agent: AgentId
  readonly transport: McpTransport
  readonly details: { supported: ReadonlyArray<McpTransport>; hint: string }
  constructor(
    agent: AgentId,
    transport: McpTransport,
    details: { supported: ReadonlyArray<McpTransport>; hint: string },
  ) {
    super(
      `Agent "${agent}" does not support the "${transport}" transport ` +
        `(supported: ${details.supported.join(', ')}). ${details.hint}`,
    )
    this.name = 'UnsupportedTransportError'
    this.agent = agent
    this.transport = transport
    this.details = details
  }
}

export class UnresolvedConfigPathError extends McpManagerError {
  readonly agent: AgentId
  constructor(agent: AgentId, reason: string) {
    super(`Cannot resolve config path for agent "${agent}": ${reason}`)
    this.name = 'UnresolvedConfigPathError'
    this.agent = agent
  }
}

/**
 * Thrown by `link()` (and `planLink` at the pure layer) when the target
 * agent's config file location is not writable-safely: neither the file
 * nor its parent directory exists on disk. The agent has either not been
 * installed or has been installed but never launched, so the config
 * directory does not exist yet.
 *
 * Consumers precheck with `isInstalled({agents})` to avoid this error;
 * or catch it and surface the install prompt to the user.
 */
export class AgentNotInstalledError extends McpManagerError {
  readonly agent: AgentId
  readonly configPath: string
  readonly parentDir: string
  constructor(agent: AgentId, configPath: string, parentDir: string) {
    super(
      `Agent "${agent}" does not appear to be installed on this machine. ` +
        `The library needs "${configPath}" or its parent directory "${parentDir}" ` +
        `to exist before it can write an MCP entry. ` +
        `Install ${agent} and launch it at least once, or pass an explicit ` +
        `"configPath" to write to a custom location.`,
    )
    this.name = 'AgentNotInstalledError'
    this.agent = agent
    this.configPath = configPath
    this.parentDir = parentDir
  }
}
