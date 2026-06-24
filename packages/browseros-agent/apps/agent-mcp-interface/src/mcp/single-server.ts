/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single MCP server for the v2 cockpit. Replaces the per-slug pattern
 * in `manager.ts` with one shared `McpServer` + one shared transport
 * mounted at `POST /cockpit/mcp`. The transport runs in stateful
 * mode (`sessionIdGenerator` returns a fresh uuid per initialize) so
 * many agents can share the same endpoint without colliding on
 * session state. Identity is captured at handshake via
 * `onsessioninitialized` and dropped at `onsessionclosed`; per-tool
 * dispatch reads it back through `extra.sessionId` inside the
 * register wrapper.
 *
 * Construction is lazy and idempotent. The route handler calls
 * `getSingleMcpInstance()` per request and reuses the same instance;
 * `server.connect(transport)` happens exactly once.
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { WebStandardStreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js'
import { logger } from '../lib/logger'
import { type ClientIdentity, identityService } from '../lib/mcp-session'
import { registerBrowserToolsForSingleServer } from './register'

const SERVER_NAME = 'browseros-agent-mcp-interface'
const SERVER_TITLE = 'BrowserOS'
const SERVER_VERSION = '0.0.1'

interface SingleMcpInstance {
  server: McpServer
  transport: WebStandardStreamableHTTPServerTransport
  /** Mirror of the SDK's per-session capture; passed into the register wrapper. */
  resolveIdentity(sessionId: string | undefined): ClientIdentity | null
}

let cached: SingleMcpInstance | null = null

export function getSingleMcpInstance(): SingleMcpInstance {
  if (cached) return cached

  const server = new McpServer({
    name: SERVER_NAME,
    title: SERVER_TITLE,
    version: SERVER_VERSION,
  })

  const transport = new WebStandardStreamableHTTPServerTransport({
    sessionIdGenerator: () => crypto.randomUUID(),
    enableJsonResponse: true,
    onsessioninitialized(sessionId) {
      const clientInfo = server.server.getClientVersion()
      identityService.registerInitialize({
        sessionId,
        clientInfo: {
          name: clientInfo?.name,
          version: clientInfo?.version,
          title: clientInfo?.title,
        },
      })
      logger.info('cockpit v2 mcp session opened', {
        sessionId,
        clientName: clientInfo?.name ?? '',
      })
    },
    onsessionclosed(sessionId) {
      identityService.dropSession(sessionId)
      logger.info('cockpit v2 mcp session closed', { sessionId })
    },
  })

  const instance: SingleMcpInstance = {
    server,
    transport,
    resolveIdentity(sessionId) {
      if (!sessionId) return null
      return identityService.getIdentity(sessionId)
    },
  }

  registerBrowserToolsForSingleServer(server, instance.resolveIdentity)

  // Connect once. Subsequent `transport.handleRequest` calls
  // multiplex through the same connection. The SDK rejects a
  // second `connect` so we must guard this.
  void server.connect(transport).catch((err) => {
    logger.error('cockpit v2 server.connect failed', {
      error: err instanceof Error ? err.message : String(err),
    })
  })

  cached = instance
  return instance
}

/**
 * Test-only escape hatch. Drops the cached instance so a subsequent
 * `getSingleMcpInstance` call rebuilds. Production must never call
 * this; the connect cost is small but a duplicate transport leaks.
 */
export function resetSingleMcpInstanceForTesting(): void {
  cached = null
}
