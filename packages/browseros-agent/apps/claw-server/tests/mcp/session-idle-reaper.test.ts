/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pins the idle-reaper behaviour for the single MCP endpoint.
 * Pre-fix, sessions stayed in the in-memory map forever unless the
 * client sent an explicit `DELETE /mcp`; codex and most other
 * clients do not, so `agent_session_ends` never got a row and the
 * session state leaked. The sweeper writes the same end
 * row as the explicit DELETE path, on the same timeout boundary that
 * `services/tasks.ts:deriveStatus` already used at read time.
 *
 * `sweepIdleSessions(now)` is exported so tests can drive a
 * deterministic clock without manipulating timers.
 */

import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { eq } from 'drizzle-orm'
import { agentSessionEnds } from '../../src/modules/db/schema/schema'

interface GroupCall {
  toolName: string
  args: Record<string, unknown>
  signal?: AbortSignal
}

const groupCalls: GroupCall[] = []
const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (
    tool: { name: string },
    args: Record<string, unknown>,
    context: { signal?: AbortSignal },
  ) => {
    groupCalls.push({ toolName: tool.name, args, signal: context.signal })
    return { isError: false, content: [{ type: 'text', text: 'ok' }] }
  },
}))

const { ownershipStore } = await import('../../src/domain/ownership')
const { env } = await import('../../src/env')
const { setBrowserSession } = await import('../../src/lib/browser-session')
const { agentKeyFromClient, identityService } = await import(
  '../../src/lib/mcp-session'
)
const { maybeRequestSessionNaming, resetSessionNamingForTests } = await import(
  '../../src/mcp/session-naming'
)
const {
  getSessionRefsForTesting,
  resetSingleMcpInstanceForTesting,
  setLastActivityForTesting,
  sweepIdleSessions,
} = await import('../../src/mcp/single-server')
const { getAuditDb, resetAuditDbForTesting, setAuditDbForTesting } =
  await import('../../src/modules/db/db')
const { dispatchCancellation } = await import(
  '../../src/services/dispatch-cancellation'
)
const app = (await import('../../src/server')).default

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
  const identity = identityService.getIdentity(sessionId)
  if (!identity) throw new Error('no identity registered')
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
    ownershipStore.clear()
    dispatchCancellation.clear()
    resetSessionNamingForTests()
    groupCalls.length = 0
    setBrowserSession(null)
    env.sessionIdleMs = 50
  })
  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    ownershipStore.clear()
    dispatchCancellation.clear()
    resetSessionNamingForTests()
    groupCalls.length = 0
    setBrowserSession(null)
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

  test('last session for a key collapses its group without closing it', async () => {
    const { client, sessionId } = await connect('claude-code')
    const identity = identityService.getIdentity(sessionId)
    if (!identity) throw new Error('identity missing')
    const key = agentKeyFromClient(identity)
    ownershipStore.setGroup(key, {
      id: 'G1',
      windowId: 1,
      color: 'red',
      title: key,
      titleExplicit: false,
      collapsed: false,
    })
    setBrowserSession({} as never)

    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    expect(sweepIdleSessions(Date.now())).toEqual([sessionId])
    await Promise.resolve()

    expect(groupCalls).toHaveLength(1)
    expect(groupCalls[0]?.args).toEqual({
      action: 'update',
      groupId: 'G1',
      collapsed: true,
    })
    expect(groupCalls[0]?.signal).toBeInstanceOf(AbortSignal)
    expect(groupCalls.some((call) => call.args.action === 'close')).toBe(false)
    expect(ownershipStore.groupOf(key)?.collapsed).toBe(true)
    expect(ownershipStore.size()).toBe(1)
    await client.close()
  })

  test('reap collapses then forgets fallback-keyed ownership', async () => {
    const initialSize = ownershipStore.size()
    const { client, sessionId } = await connect('!!!')
    const identity = identityService.getIdentity(sessionId)
    if (!identity) throw new Error('identity missing')
    const key = agentKeyFromClient(identity)
    ownershipStore.claimPage(key, 7)
    ownershipStore.setGroup(key, {
      id: 'G-fallback',
      windowId: 1,
      color: 'red',
      title: key,
      titleExplicit: false,
      collapsed: false,
    })
    setBrowserSession({} as never)

    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    expect(sweepIdleSessions(Date.now())).toEqual([sessionId])
    await Promise.resolve()

    expect(groupCalls).toHaveLength(1)
    expect(groupCalls[0]?.args).toEqual({
      action: 'update',
      groupId: 'G-fallback',
      collapsed: true,
    })
    expect(groupCalls.some((call) => call.args.action === 'close')).toBe(false)
    expect(ownershipStore.size()).toBe(initialSize)
    expect(ownershipStore.pagesOf(key)).toEqual(new Set())
    expect(ownershipStore.ownerOf(7)).toBeNull()
    expect(ownershipStore.groupOf(key)).toBeNull()
    await client.close()
  })

  test('does not collapse while another same-key session is live', async () => {
    const first = await connect('claude-code')
    const second = await connect('claude-code')
    const identity = identityService.getIdentity(first.sessionId)
    if (!identity) throw new Error('identity missing')
    const key = agentKeyFromClient(identity)
    ownershipStore.setGroup(key, {
      id: 'G1',
      windowId: 1,
      color: 'red',
      title: key,
      titleExplicit: false,
      collapsed: false,
    })
    setBrowserSession({} as never)

    setLastActivityForTesting(first.sessionId, Date.now() - 10_000)
    expect(sweepIdleSessions(Date.now())).toEqual([first.sessionId])
    await Promise.resolve()

    expect(groupCalls).toEqual([])
    expect(ownershipStore.groupOf(key)?.collapsed).toBe(false)
    ownershipStore.clear()
    await first.client.close()
    await second.client.close()
  })

  test('teardown aborts every in-flight dispatch for the session', async () => {
    const { client, sessionId } = await connect('claude-code')
    const first = new AbortController()
    const second = new AbortController()
    dispatchCancellation.register(sessionId, first)
    dispatchCancellation.register(sessionId, second)

    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    expect(sweepIdleSessions(Date.now())).toEqual([sessionId])

    expect(first.signal.aborted).toBe(true)
    expect(second.signal.aborted).toBe(true)
    expect(first.signal.reason).toBe('MCP session ended')
    await client.close()
  })

  test('teardown cancels a hanging naming elicitation cleanly', async () => {
    const { client, sessionId } = await connect('claude-code')
    let elicitationAborted = false
    const naming = maybeRequestSessionNaming({
      sessionId,
      requestId: 1,
      server: {
        getClientCapabilities: () => ({ elicitation: {} }),
        elicitInput: (_params, options) =>
          new Promise((_, reject) => {
            options.signal?.addEventListener(
              'abort',
              () => {
                elicitationAborted = true
                reject(new DOMException('aborted', 'AbortError'))
              },
              { once: true },
            )
          }),
      },
    })

    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    expect(sweepIdleSessions(Date.now())).toEqual([sessionId])
    await expect(naming).resolves.toBeUndefined()
    expect(elicitationAborted).toBe(true)
    await client.close()
  })
})
