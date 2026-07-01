/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pins the idle-reaper behaviour for the v2 MCP single endpoint.
 * Pre-fix, sessions stayed in the in-memory map forever unless the
 * client sent an explicit `DELETE /mcp`; codex and most other
 * clients do not, so `agent_session_ends` never got a row and the
 * agent's tab group leaked. The sweeper here writes the same end
 * row and fires the same tab-group close that the explicit DELETE
 * path always did, on the same `IDLE_TIMEOUT_MS` boundary that
 * `services/tasks.ts:deriveStatus` already used at read time.
 *
 * `sweepIdleSessions(now)` is exported so tests can drive a
 * deterministic clock without manipulating timers.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { eq } from 'drizzle-orm'
import { env } from '../../src/env'
import { tabGroupTracker } from '../../src/lib/agent-tab-groups'
import { identityService } from '../../src/lib/mcp-session'
import {
  getSessionRefsForTesting,
  resetSingleMcpInstanceForTesting,
  setLastActivityForTesting,
  sweepIdleSessions,
} from '../../src/mcp/single-server'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { agentSessionEnds } from '../../src/modules/db/schema/schema'
import app from '../../src/server'

async function connect(clientName: string) {
  const transport = new StreamableHTTPClientTransport(
    new URL('http://localhost/mcp'),
    {
      fetch: ((input, init) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  const client = new Client(
    { name: clientName, version: '0.0.1' },
    { capabilities: {} },
  )
  await client.connect(transport)
  const sessionId = transport.sessionId
  if (!sessionId) throw new Error('no session id assigned')
  return { client, sessionId }
}

function endRowsFor(sessionId: string): Array<{ kind: string }> {
  return getAuditDb()
    .select({ kind: agentSessionEnds.kind })
    .from(agentSessionEnds)
    .where(eq(agentSessionEnds.sessionId, sessionId))
    .all()
}

const ORIGINAL_IDLE = env.sessionIdleMs

describe('sweepIdleSessions', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    tabGroupTracker.reset()
    env.sessionIdleMs = 50
  })
  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    tabGroupTracker.reset()
    env.sessionIdleMs = ORIGINAL_IDLE
    resetAuditDbForTesting()
  })

  test('reaps a session whose lastActivityAt is older than the idle window', async () => {
    {
      const { client, sessionId } = await connect('codex-mcp-client')
      expect(identityService.size()).toBe(1)
      // Backdate the session well past env.sessionIdleMs and sweep
      // against a `now` that is current. The reaper should drop it.
      setLastActivityForTesting(sessionId, Date.now() - 10_000)
      const swept = sweepIdleSessions(Date.now())
      expect(swept).toEqual([sessionId])
      expect(identityService.getIdentity(sessionId)).toBeNull()
      // agent_session_ends has the closed row.
      const rows = endRowsFor(sessionId)
      expect(rows).toHaveLength(1)
      expect(rows[0]?.kind).toBe('closed')
      await client.close()
    }
  })

  test('does NOT reap a session whose lastActivityAt is recent', async () => {
    {
      const { client, sessionId } = await connect('codex-mcp-client')
      // Recent activity: do not backdate. Sweep with `now` only
      // slightly ahead; the gap is below env.sessionIdleMs (50ms).
      const swept = sweepIdleSessions(Date.now() + 10)
      expect(swept).toEqual([])
      expect(identityService.getIdentity(sessionId)).not.toBeNull()
      expect(endRowsFor(sessionId)).toEqual([])
      await client.close()
    }
  })

  test('a second sweep against the same idle session is a no-op (idempotent)', async () => {
    {
      const { client, sessionId } = await connect('codex-mcp-client')
      setLastActivityForTesting(sessionId, Date.now() - 10_000)
      expect(sweepIdleSessions(Date.now())).toEqual([sessionId])
      // Second sweep: nothing to reap. cleanupSessionState is
      // gated on `sessions.has(sessionId)` so no double-write.
      expect(sweepIdleSessions(Date.now())).toEqual([])
      expect(endRowsFor(sessionId)).toHaveLength(1)
      await client.close()
    }
  })

  test('reaping decrements the tab-group tracker so the agent group can be closed', async () => {
    {
      const { client, sessionId } = await connect('codex-mcp-client')
      // The initialized notification called tabGroupTracker
      // .incrementSession('codex-mcp-client'). After reap, the
      // ref count goes back to 0 and getByAgentId returns null
      // (closeAgentTabGroupForAgent calls decrementSession which
      // removes the record when refCount hits 0).
      expect(tabGroupTracker.getByAgentId('codex-mcp-client')).not.toBeNull()
      setLastActivityForTesting(sessionId, Date.now() - 10_000)
      sweepIdleSessions(Date.now())
      expect(tabGroupTracker.getByAgentId('codex-mcp-client')).toBeNull()
      await client.close()
    }
  })

  test('two sessions: only the idle one is reaped, the active stays', async () => {
    {
      const a = await connect('codex-mcp-client')
      const b = await connect('claude-code')
      setLastActivityForTesting(a.sessionId, Date.now() - 10_000)
      // b stays fresh.
      const swept = sweepIdleSessions(Date.now())
      expect(swept).toEqual([a.sessionId])
      expect(identityService.getIdentity(a.sessionId)).toBeNull()
      expect(identityService.getIdentity(b.sessionId)).not.toBeNull()
      await a.client.close()
      await b.client.close()
    }
  })

  test('reap calls transport.close() so long-lived SSE streams do not leak', async () => {
    // Without transport.close(), SSE GET streams held by clients
    // like codex-mcp-client / claude-code stay open server-side
    // until the client's TCP connection eventually drops. Assert
    // close() actually fires on reap by installing spies on the
    // transport and server before we backdate + sweep.
    {
      const { client, sessionId } = await connect('codex-mcp-client')
      const refs = getSessionRefsForTesting(sessionId)
      expect(refs).not.toBeNull()
      if (!refs) throw new Error('refs must exist')
      let transportClosed = 0
      let serverClosed = 0
      const origTransportClose = refs.transport.close.bind(refs.transport)
      const origServerClose = refs.server.close.bind(refs.server)
      refs.transport.close = async () => {
        transportClosed++
        return origTransportClose()
      }
      refs.server.close = async () => {
        serverClosed++
        return origServerClose()
      }

      setLastActivityForTesting(sessionId, Date.now() - 10_000)
      const swept = sweepIdleSessions(Date.now())
      expect(swept).toEqual([sessionId])
      expect(transportClosed).toBe(1)
      expect(serverClosed).toBe(1)
      await client.close()
    }
  })
})
