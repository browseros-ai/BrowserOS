/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

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
import {
  BROWSEROS_TOOL_LEASE_HEADER,
  type BrowserMcpModule,
  InvalidBrowserToolLeaseError,
} from '../services/mcp/browser-mcp-module'
import type { Env } from '../types'

export { BROWSEROS_TOOL_LEASE_HEADER }

type CreateMcpTransportFn = (
  options: ConstructorParameters<
    typeof WebStandardStreamableHTTPServerTransport
  >[0],
) => InstanceType<typeof WebStandardStreamableHTTPServerTransport>

interface McpRouteDeps {
  browserMcp: Pick<BrowserMcpModule, 'createMcpServer' | 'validateLeaseToken'>
  createMcpTransport?: CreateMcpTransportFn
}

interface McpRequestLogContext extends Record<string, unknown> {
  leased: boolean
  requestedReadOnly: boolean
  includeStructuredContent: boolean
}

/** Creates the Hono routes that expose BrowserOS as a request-scoped MCP server. */
export function createMcpRoutes(deps: McpRouteDeps) {
  const app = new Hono<Env>()
  app.use('/*', rejectBrowserFetch())
  const makeMcpTransport =
    deps.createMcpTransport ??
    ((options) => new WebStandardStreamableHTTPServerTransport(options))

  // The URL selects response/catalog shape. Conversation permissions and
  // mutable context are resolved from the opaque, server-owned lease.
  function readScope(req: Request) {
    const url = new URL(req.url)
    const leaseToken =
      req.headers.get(BROWSEROS_TOOL_LEASE_HEADER)?.trim() || undefined
    const includeStructuredContent = url.searchParams.get('structured') === '1'
    const requestedReadOnly = url.searchParams.get('read_only') === '1'
    const logContext: McpRequestLogContext = {
      leased: leaseToken !== undefined,
      requestedReadOnly,
      includeStructuredContent,
    }
    return {
      leaseToken,
      includeStructuredContent,
      requestedReadOnly,
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
          leaseToken: undefined,
          includeStructuredContent: false,
          requestedReadOnly: false,
        }
    return deps.browserMcp.createMcpServer({
      leaseToken: scope.leaseToken,
      requestedReadOnly: scope.requestedReadOnly,
      includeStructuredContent: scope.includeStructuredContent,
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
    const { leaseToken, logContext } = readScope(raw)
    if (leaseToken) {
      try {
        // Modern createMcpHandler converts factory exceptions into a generic
        // 500, so preserve the module's typed authorization result at ingress.
        deps.browserMcp.validateLeaseToken(leaseToken)
      } catch (error) {
        if (error instanceof InvalidBrowserToolLeaseError) {
          return invalidLeaseResponse(error, logContext)
        }
        throw error
      }
    }

    metrics.log('mcp.request', { leased: leaseToken !== undefined })
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
      if (error instanceof InvalidBrowserToolLeaseError) {
        return invalidLeaseResponse(error, logContext)
      }
      Sentry.withScope((scope) => {
        scope.setTag('route', 'mcp')
        scope.setTag('leased', leaseToken !== undefined)
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

function invalidLeaseResponse(
  error: InvalidBrowserToolLeaseError,
  logContext: McpRequestLogContext,
): Response {
  logger.warn('MCP request rejected for invalid lease', logContext)
  return Response.json(
    {
      jsonrpc: '2.0',
      error: { code: -32001, message: error.message },
      id: null,
    },
    { status: 401 },
  )
}
