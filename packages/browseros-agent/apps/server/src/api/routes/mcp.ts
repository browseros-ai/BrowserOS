/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import {
  createMcpHandler,
  isLegacyRequest,
  type McpRequestContext,
  WebStandardStreamableHTTPServerTransport,
} from '@modelcontextprotocol/server'
import { Hono } from 'hono'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { Sentry } from '../../lib/sentry'
import { rejectBrowserFetch } from '../middleware/reject-browser-fetch'
import type { KlavisService } from '../services/klavis'
import { createMcpServer } from '../services/mcp/mcp-server'
import type { ServerActivity } from '../services/server-activity'
import type { Env } from '../types'

export const MANAGED_MCP_SERVERS_HEADER = 'X-BrowserOS-Managed-Mcp-Servers'

type CreateMcpServerFn = typeof createMcpServer
type CreateMcpTransportFn = (
  options: ConstructorParameters<
    typeof WebStandardStreamableHTTPServerTransport
  >[0],
) => InstanceType<typeof WebStandardStreamableHTTPServerTransport>

interface McpRouteDeps {
  version: string
  browserSession: BrowserSession
  klavis?: KlavisService
  createMcpServer?: CreateMcpServerFn
  createMcpTransport?: CreateMcpTransportFn
  activity?: ServerActivity
}

interface McpRequestLogContext extends Record<string, unknown> {
  scopeId: string
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
  app.use('/*', rejectBrowserFetch())
  const makeMcpServer = deps.createMcpServer ?? createMcpServer
  const makeMcpTransport =
    deps.createMcpTransport ??
    ((options) => new WebStandardStreamableHTTPServerTransport(options))

  // Reads the per-request BrowserOS scope from headers + query. Used by the
  // POST handler (metrics/logging on every request, including rejected ones)
  // and by the factory (per-request server construction).
  function readScope(req: Request) {
    const url = new URL(req.url)
    const scopeId = req.headers.get('X-BrowserOS-Scope-Id') || 'ephemeral'
    const includeStructuredContent = url.searchParams.get('structured') === '1'
    const defaultWindowId = parseOptionalNumber(
      req.headers.get('X-BrowserOS-Default-Window-Id') ?? undefined,
    )
    const defaultTabGroupId =
      req.headers.get('X-BrowserOS-Default-Tab-Group-Id') ?? undefined
    const selectedServerNames = parseManagedMcpServersHeader(
      req.headers.get(MANAGED_MCP_SERVERS_HEADER) ?? undefined,
    )
    const logContext: McpRequestLogContext = {
      scopeId,
      selectedServerNames,
      selectedServerCount: selectedServerNames.length,
      defaultWindowId,
      defaultTabGroupId,
    }
    return {
      scopeId,
      includeStructuredContent,
      defaultWindowId,
      defaultTabGroupId,
      selectedServerNames,
      logContext,
    }
  }

  // One factory backs both eras: createMcpHandler's modern leg and the
  // hand-wired legacy JSON leg. The factory receives the request, so per-request
  // header scoping is preserved.
  const buildServer = (ctx: McpRequestContext) => {
    const scope = ctx.requestInfo
      ? readScope(ctx.requestInfo)
      : {
          includeStructuredContent: false,
          defaultWindowId: undefined,
          defaultTabGroupId: undefined,
          selectedServerNames: [] as string[],
        }
    return makeMcpServer({
      version: deps.version,
      browserSession: deps.browserSession,
      klavis: deps.klavis,
      connectorScope: { selectedServerNames: scope.selectedServerNames },
      defaultWindowId: scope.defaultWindowId,
      defaultTabGroupId: scope.defaultTabGroupId,
      includeStructuredContent: scope.includeStructuredContent,
      activity: deps.activity,
    })
  }

  // Modern (2026-07-28) leg. `legacy: 'reject'` keeps 2025-era traffic off this
  // handler; isLegacyRequest routes those to the legacy JSON transport below,
  // which preserves the existing single-JSON (non-SSE) response shape that the
  // internal ACP client depends on.
  const modern = createMcpHandler(buildServer, { legacy: 'reject' })

  app.get('/', (c) =>
    c.json({
      status: 'ok',
      message: 'MCP server is running. Use POST to interact.',
    }),
  )

  app.post('/', async (c) => {
    const raw = c.req.raw
    const { scopeId, logContext } = readScope(raw)
    metrics.log('mcp.request', { scopeId })
    logger.debug('MCP request received', logContext)

    try {
      // Legacy (2025-era) stays byte-for-byte identical: hand-wired stateless
      // transport with enableJsonResponse, so the internal ACP client keeps its
      // single-JSON responses. isLegacyRequest reads a clone, leaving `raw`
      // consumable by whichever leg serves the request.
      if (await isLegacyRequest(raw)) {
        const mcpServer = buildServer({ era: 'legacy', requestInfo: raw })
        const transport = makeMcpTransport({
          sessionIdGenerator: undefined,
          enableJsonResponse: true,
        })
        await mcpServer.connect(transport)
        logger.debug('MCP request transport connected', logContext)
        const response = await transport.handleRequest(raw)
        logger.debug('MCP request handled', {
          ...logContext,
          status: response?.status,
        })
        return response
      }

      // Modern (2026-07-28) requests: server/discover + stateless dispatch.
      const response = await modern.fetch(raw)
      logger.debug('MCP request handled', {
        ...logContext,
        status: response.status,
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
