/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { StreamableHTTPTransport } from '@hono/mcp'
import { Hono } from 'hono'
import type { Browser } from '../../browser/browser'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { Sentry } from '../../lib/sentry'
import { getMonitoringService } from '../../monitoring/service'
import type { ToolRegistry } from '../../tools/tool-registry'
import type { GlobalAclPolicyService } from '../services/acl/global-acl-policy'
import { resolveAclPolicyForMcpRequest } from '../services/acl/resolve-acl-policy'
import type { KlavisProxyRef } from '../services/klavis/strata-proxy'
import { createMcpServer } from '../services/mcp/mcp-server'
import type { Env } from '../types'

interface McpRouteDeps {
  version: string
  registry: ToolRegistry
  browser: Browser
  executionDir: string
  resourcesDir: string
  policyService: GlobalAclPolicyService
  klavisRef?: KlavisProxyRef
}

export function createMcpRoutes(deps: McpRouteDeps) {
  const app = new Hono<Env>()

  app.get('/', (c) =>
    c.json({
      status: 'ok',
      message: 'MCP server is running. Use POST to interact.',
    }),
  )

  app.post('/', async (c) => {
    const scopeId = c.req.header('X-BrowserOS-Scope-Id') || 'ephemeral'
    const agentId =
      c.req.query('agentId') ??
      c.req.header('X-BrowserOS-Agent-Id') ??
      undefined
    metrics.log('mcp.request', { scopeId })
    const aclRules = await resolveAclPolicyForMcpRequest({
      policyService: deps.policyService,
    })
    const monitoringSessionId = agentId
      ? getMonitoringService().getActiveSessionId(agentId)
      : undefined
    const observer =
      monitoringSessionId && agentId
        ? getMonitoringService().createObserver(monitoringSessionId, agentId)
        : undefined

    // Per-request server + transport: no shared state, no race conditions,
    // no ID collisions. Required by MCP SDK 1.26.0+ security fix (GHSA-345p-7cg4-v4c7).
    const mcpServer = createMcpServer({
      ...deps,
      aclRules,
      observer,
    })
    const transport = new StreamableHTTPTransport({
      sessionIdGenerator: undefined,
      enableJsonResponse: true,
    })

    try {
      await mcpServer.connect(transport)
      return transport.handleRequest(c)
    } catch (error) {
      Sentry.withScope((scope) => {
        scope.setTag('route', 'mcp')
        scope.setTag('scopeId', scopeId)
        if (agentId) {
          scope.setTag('agentId', agentId)
        }
        Sentry.captureException(error)
      })
      logger.error('Error handling MCP request', {
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
