/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 single MCP endpoint. Every agent connects to the same public
 * URL (`POST /cockpit/mcp`); the SDK's stateful Streamable HTTP
 * transport supports one session per transport instance, so we keep
 * a `sessionId -> { server, transport }` map and route each request
 * to its session's transport. Identity is captured on the client's
 * InitializedNotification (by then both `transport.sessionId` and
 * the server's stored `clientInfo` are set) and dropped when the
 * session ends. Tool dispatch reads identity back via
 * `extra.sessionId`, the same id the transport stamps onto the
 * session at handshake.
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { WebStandardStreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js'
import { tabGroupTracker } from '../lib/agent-tab-groups'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  type ClientIdentity,
  identityService,
} from '../lib/mcp-session'
import { closeAgentTabGroupForAgent } from '../services/tab-group-ops'
import { registerBrowserToolsForSingleServer } from './register'

const SERVER_NAME = 'browseros-agent-mcp-interface'
const SERVER_TITLE = 'BrowserOS'
const SERVER_VERSION = '0.0.1'

interface Session {
  server: McpServer
  transport: WebStandardStreamableHTTPServerTransport
}

const sessions = new Map<string, Session>()

function resolveIdentity(sessionId: string | undefined): ClientIdentity | null {
  if (!sessionId) return null
  return identityService.getIdentity(sessionId)
}

function buildSession(): Session {
  const server = new McpServer({
    name: SERVER_NAME,
    title: SERVER_TITLE,
    version: SERVER_VERSION,
  })

  registerBrowserToolsForSingleServer(server, resolveIdentity)

  const transport = new WebStandardStreamableHTTPServerTransport({
    sessionIdGenerator: () => crypto.randomUUID(),
    enableJsonResponse: true,
    onsessionclosed(sessionId) {
      // Read identity BEFORE dropping it so the cleanup hook can
      // resolve the agentId.
      const identity = identityService.getIdentity(sessionId)
      if (identity) {
        const { agentId } = agentIdentityFromClient(identity)
        const browserSession = getBrowserSession()
        if (browserSession) {
          void closeAgentTabGroupForAgent({
            agentId,
            session: browserSession,
          })
        }
      }
      sessions.delete(sessionId)
      identityService.dropSession(sessionId)
      logger.info('cockpit v2 mcp session closed', { sessionId })
    },
  })

  // `oninitialized` fires on the InitializedNotification, by which
  // point both `transport.sessionId` and `server._clientVersion` are
  // set. Reading clientInfo any earlier (eg from the transport's
  // `onsessioninitialized`) gets undefined because the server has
  // not yet processed the initialize request body.
  server.server.oninitialized = () => {
    const sessionId = transport.sessionId
    if (!sessionId) return
    const clientInfo = server.server.getClientVersion()
    const identity = identityService.registerInitialize({
      sessionId,
      clientInfo: {
        name: clientInfo?.name,
        version: clientInfo?.version,
        title: clientInfo?.title,
      },
    })
    // Bump the tab-group tracker's per-agentId ref count so the
    // close path only deletes the group when the last session for
    // this agent ends.
    const { agentId } = agentIdentityFromClient(identity)
    tabGroupTracker.incrementSession(agentId)
    logger.info('cockpit v2 mcp session opened', {
      sessionId,
      clientName: clientInfo?.name ?? '',
    })
  }

  return { server, transport }
}

/**
 * Routes one incoming request to its session-bound transport. An
 * initialize call without an `mcp-session-id` header mints a fresh
 * (server, transport) pair, lets the SDK assign the session id, and
 * persists the pair in the session map. Subsequent requests on the
 * same session land back on the same pair via the header.
 */
export async function handleSingleMcpRequest(
  request: Request,
): Promise<Response> {
  const headerSessionId = request.headers.get('mcp-session-id')
  if (headerSessionId) {
    const existing = sessions.get(headerSessionId)
    if (existing) {
      return existing.transport.handleRequest(request)
    }
    // Unknown session id. Reject upfront with a structured 404 so
    // the client knows to drop the stale header and re-initialize.
    // Falling through to buildSession() would attach a fresh server
    // + transport that the SDK immediately rejects in validateSession
    // (no matching session id), leaving the pair connected and
    // un-tracked: a per-request leak.
    return new Response(
      JSON.stringify({
        error: 'unknown mcp-session-id',
        hint: 'drop the mcp-session-id header and send an initialize request to start a new session',
      }),
      { status: 404, headers: { 'content-type': 'application/json' } },
    )
  }

  const session = buildSession()
  await session.server.connect(session.transport)
  const response = await session.transport.handleRequest(request)
  const assignedId = session.transport.sessionId
  if (assignedId) {
    sessions.set(assignedId, session)
  }
  return response
}

/**
 * Test-only escape hatch. Drops every cached session so a subsequent
 * request rebuilds from scratch.
 */
export function resetSingleMcpInstanceForTesting(): void {
  sessions.clear()
}
