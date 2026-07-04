/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { StreamableHTTPTransport } from '@hono/mcp'
import { Hono } from 'hono'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { Sentry } from '../../lib/sentry'
import type { KlavisService } from '../services/klavis'
import { createMcpServer } from '../services/mcp/mcp-server'
import type { ServerActivity } from '../services/server-activity'
import type { Env } from '../types'

export const MANAGED_MCP_SERVERS_HEADER = 'X-BrowserOS-Managed-Mcp-Servers'
export const REMOTE_AGENT_HARNESS_MCP_SOURCE = 'remote-agent-harness'

type CreateMcpServerFn = typeof createMcpServer
type CreateMcpTransportFn = (
  options: ConstructorParameters<typeof StreamableHTTPTransport>[0],
) => InstanceType<typeof StreamableHTTPTransport>

interface McpRouteDeps {
  version: string
  browserSession: BrowserSession
  klavis?: KlavisService
  executionDir: string
  createMcpServer?: CreateMcpServerFn
  createMcpTransport?: CreateMcpTransportFn
  activity?: ServerActivity
}

interface McpRequestLogContext extends Record<string, unknown> {
  scopeId: string
  source?: string
  remoteAgentHarness: boolean
  selectedServerNames: string[]
  selectedServerCount: number
  defaultWindowId?: number
  defaultTabGroupId?: string
}

function parseOptionalNumber(value: string | undefined): number | undefined {
  if (value === undefined) return undefined
  const n = Number(value)
  // CDP window ids are integers; `Number.isFinite('1.5')` would be true
  // and silently route to a non-integer that CDP rejects with an opaque
  // protocol error. Require an integer at the parse boundary.
  return Number.isInteger(n) ? n : undefined
}

/** Parses the internal ACP managed-connector scope header. */
export function parseManagedMcpServersHeader(
  value: string | undefined,
): string[] {
  if (!value?.trim()) {
    return []
  }
  const out: string[] = []
  for (const part of value.split(',')) {
    if (!part) continue
    try {
      const decoded = decodeURIComponent(part)
      if (decoded) {
        out.push(decoded)
      }
    } catch {
      return []
    }
  }
  return out
}

/** Creates the Hono routes that expose BrowserOS as a request-scoped MCP server. */
export function createMcpRoutes(deps: McpRouteDeps) {
  const app = new Hono<Env>()
  const makeMcpServer = deps.createMcpServer ?? createMcpServer
  const makeMcpTransport =
    deps.createMcpTransport ??
    ((options) => new StreamableHTTPTransport(options))
  const remoteAgentHarness = {
    outputFileAccess: createBrowserOutputFileAccess(),
  }

  app.get('/', (c) =>
    c.json({
      status: 'ok',
      message: 'MCP server is running. Use POST to interact.',
    }),
  )

  app.post('/', async (c) => {
    const scopeId = c.req.header('X-BrowserOS-Scope-Id') || 'ephemeral'
    metrics.log('mcp.request', { scopeId })
    const source = c.req.query('source') || undefined

    const defaultWindowId = parseOptionalNumber(
      c.req.header('X-BrowserOS-Default-Window-Id'),
    )
    const defaultTabGroupId =
      c.req.header('X-BrowserOS-Default-Tab-Group-Id') ?? undefined
    const selectedServerNames = parseManagedMcpServersHeader(
      c.req.header(MANAGED_MCP_SERVERS_HEADER),
    )

    const harness =
      source === REMOTE_AGENT_HARNESS_MCP_SOURCE
        ? remoteAgentHarness
        : undefined
    const logContext: McpRequestLogContext = {
      scopeId,
      source,
      remoteAgentHarness: Boolean(harness),
      selectedServerNames,
      selectedServerCount: selectedServerNames.length,
      defaultWindowId,
      defaultTabGroupId,
    }

    logger.debug('MCP request received', logContext)

    // Per-request server + transport: no shared state, no race conditions,
    // no ID collisions. Required by MCP SDK 1.26.0+ security fix (GHSA-345p-7cg4-v4c7).
    const mcpServer = makeMcpServer({
      version: deps.version,
      browserSession: deps.browserSession,
      klavis: deps.klavis,
      connectorScope: { selectedServerNames },
      defaultWindowId,
      defaultTabGroupId,
      executionDir: deps.executionDir,
      remoteAgentHarness: harness,
      activity: deps.activity,
    })
    const transport = makeMcpTransport({
      sessionIdGenerator: undefined,
      enableJsonResponse: true,
    })

    try {
      await mcpServer.connect(transport)
      logger.debug('MCP request transport connected', logContext)
      const response = await transport.handleRequest(c)
      logger.debug('MCP request handled', {
        ...logContext,
        status: response?.status,
      })
      return response
    } catch (error) {
      Sentry.withScope((scope) => {
        scope.setTag('route', 'mcp')
        scope.setTag('scopeId', scopeId)
        Sentry.captureException(error)
      })
      logger.error('Error handling MCP request', {
        ...logContext,
        error: error instanceof Error ? error.message : String(error),
      })

      return c.json(
        {
          jsonrpc: '2.0',
          error: { code: -32603, message: 'Internal server error' },
          id: null,
        },
        500,
      )
    }
  })

  return app
}
