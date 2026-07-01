/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 single MCP endpoint. Every agent connects to the same public
 * URL (`POST /mcp`); the SDK's stateful Streamable HTTP
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
import { env } from '../env'
import { tabGroupTracker } from '../lib/agent-tab-groups'
import { agentTabs } from '../lib/agent-tabs'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  type ClientIdentity,
  identityService,
} from '../lib/mcp-session'
import {
  recordSessionEnd,
  recordSessionStart,
} from '../services/session-events'
import { closeAgentTabGroupForAgent } from '../services/tab-group-ops'
import { registerBrowserToolsForSingleServer } from './register'

const SERVER_NAME = 'browseros-claw-server'
const SERVER_TITLE = 'BrowserOS'
const SERVER_VERSION = '0.0.1'

interface Session {
  server: McpServer
  transport: WebStandardStreamableHTTPServerTransport
  /**
   * Wall-clock of the last inbound request that landed on this
   * session. Bumped by `handleSingleMcpRequest`. The idle sweeper
   * tears down sessions whose `lastActivityAt` is older than
   * `env.sessionIdleMs`.
   */
  lastActivityAt: number
}

const sessions = new Map<string, Session>()
let sweeperHandle: ReturnType<typeof setInterval> | null = null

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
      cleanupSessionState(sessionId)
    },
  })

  transport.onerror = (err) => {
    const sessionId = transport.sessionId
    if (!sessionId) return
    recordSessionEnd({
      sessionId,
      kind: 'errored',
      reason: err instanceof Error ? err.message : String(err),
    })
  }

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
    const { agentId, slug } = agentIdentityFromClient(identity)
    tabGroupTracker.incrementSession(agentId)
    const agentLabel =
      identity.clientTitle && identity.clientTitle.length > 0
        ? identity.clientTitle
        : identity.clientName.length > 0
          ? identity.clientName
          : slug
    recordSessionStart({
      sessionId,
      agentId,
      slug,
      agentLabel,
      clientName: identity.clientName,
      clientVersion: identity.clientVersion,
    })
    logger.info('cockpit v2 mcp session opened', {
      sessionId,
      clientName: clientInfo?.name ?? '',
    })
  }

  return { server, transport, lastActivityAt: Date.now() }
}

/**
 * Shared end-of-session cleanup. The explicit DELETE path (transport
 * `onsessionclosed` callback) and the idle sweeper both call this.
 * Idempotent: if the session is already gone from the map, return
 * without firing side effects so repeated sweeps are safe.
 */
function cleanupSessionState(sessionId: string): void {
  // Grab the ref BEFORE the map delete so we can close the transport
  // + server AFTER the map is empty. Idempotent guard: if the
  // session is already gone, return without firing side effects
  // (protects against the sweeper racing with the explicit DELETE
  // path and against the reentrant callback below).
  const session = sessions.get(sessionId)
  if (!session) return
  // Read identity BEFORE dropping it so the cleanup hook can resolve
  // the agentId for the tab-group close.
  const identity = identityService.getIdentity(sessionId)
  if (identity) {
    const { agentId } = agentIdentityFromClient(identity)
    const browserSession = getBrowserSession()
    if (browserSession) {
      // Decrements the tracker AND fires the CDP close on the
      // BrowserOS side. Returns immediately if refCount > 0.
      void closeAgentTabGroupForAgent({
        agentId,
        session: browserSession,
      })
    } else {
      // No live browser session. Still decrement so the ref count
      // stays accurate; the CDP close is moot because there is no
      // browser to dispatch to.
      tabGroupTracker.decrementSession(agentId)
    }
    // Drop the per-agent tabs ledger so the next session for this
    // agentId starts empty. Symmetric with the tab-group tracker.
    agentTabs.forgetAgent(agentId)
  }
  sessions.delete(sessionId)
  identityService.dropSession(sessionId)
  recordSessionEnd({ sessionId, kind: 'closed' })
  // Close the transport + server AFTER the map delete so any
  // reentrant onsessionclosed callback that the transport fires
  // from inside its own close() sees the now-empty map and no-ops
  // via the guard at the top of this function. Without these
  // calls, long-lived SSE GET streams held by clients like
  // codex-mcp-client stay open server-side until the client's
  // TCP connection eventually drops, leaking a file descriptor
  // and per-session memory for the interval between reap and
  // organic disconnect.
  void session.transport.close().catch((err) => {
    logger.warn('session transport close threw', {
      sessionId,
      error: err instanceof Error ? err.message : String(err),
    })
  })
  void session.server.close().catch((err) => {
    logger.warn('session server close threw', {
      sessionId,
      error: err instanceof Error ? err.message : String(err),
    })
  })
  logger.info('cockpit v2 mcp session closed', { sessionId })
}

/**
 * Walks the sessions map and tears down entries older than the
 * configured idle window. Exported for unit testing; production
 * callers drive it via the `setInterval` started in
 * `ensureSweeperStarted`. Returns the ids that were swept so tests
 * can assert directly without timer manipulation.
 */
export function sweepIdleSessions(now: number): string[] {
  const idle: string[] = []
  for (const [sessionId, session] of sessions.entries()) {
    if (now - session.lastActivityAt > env.sessionIdleMs) {
      idle.push(sessionId)
    }
  }
  for (const sessionId of idle) cleanupSessionState(sessionId)
  return idle
}

function ensureSweeperStarted(): void {
  if (sweeperHandle !== null) return
  sweeperHandle = setInterval(() => {
    sweepIdleSessions(Date.now())
  }, env.sessionSweepIntervalMs)
  // The cockpit's HTTP server is the lifecycle anchor; the sweep
  // timer must not pin the bun process on its own. `unref` is
  // present on Node-style Timeout values but not part of the
  // Web-standard typing, so we feature-detect rather than assert.
  const handle = sweeperHandle as { unref?: () => void }
  handle.unref?.()
}

function stopSweeper(): void {
  if (sweeperHandle === null) return
  clearInterval(sweeperHandle)
  sweeperHandle = null
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
      existing.lastActivityAt = Date.now()
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
    ensureSweeperStarted()
  }
  return response
}

/**
 * Test-only escape hatch. Drops every cached session AND stops the
 * idle sweeper interval so subsequent tests rebuild from scratch
 * without leaking timers across cases.
 */
export function resetSingleMcpInstanceForTesting(): void {
  stopSweeper()
  sessions.clear()
}

/**
 * Test-only escape hatch. Backdates a session's lastActivityAt so
 * the sweeper sees it as idle without the test having to sleep.
 * Returns true if the session existed.
 */
export function setLastActivityForTesting(
  sessionId: string,
  ms: number,
): boolean {
  const session = sessions.get(sessionId)
  if (!session) return false
  session.lastActivityAt = ms
  return true
}

/**
 * Test-only escape hatch. Returns the transport + server refs for a
 * cached session so a test can install a spy on `.close()` before
 * driving the sweeper. Returns null when the session id is unknown.
 */
export function getSessionRefsForTesting(sessionId: string): {
  transport: WebStandardStreamableHTTPServerTransport
  server: McpServer
} | null {
  const session = sessions.get(sessionId)
  if (!session) return null
  return { transport: session.transport, server: session.server }
}
