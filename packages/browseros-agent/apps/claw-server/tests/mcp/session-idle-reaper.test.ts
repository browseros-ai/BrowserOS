/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { eq } from 'drizzle-orm'
import {
  agentSessionEnds,
  agentSessionStarts,
} from '../../src/modules/db/schema/schema'

interface GroupCall {
  toolName: string
  args: Record<string, unknown>
  signal?: AbortSignal
}

const groupCalls: GroupCall[] = []
let nextGroupErrorAction: string | null = null
let tabsNewPage: number | null = null
let pendingRootGroupCreate: Promise<{
  isError: false
  content: Array<{ type: 'text'; text: string }>
  structuredContent: { group: { groupId: string; windowId: number } }
}> | null = null
let pendingGroupClose: Promise<{
  isError: false
  content: Array<{ type: 'text'; text: string }>
}> | null = null
const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (
    tool: { name: string },
    args: Record<string, unknown>,
    context: { signal?: AbortSignal },
  ) => {
    groupCalls.push({ toolName: tool.name, args, signal: context.signal })
    if (tool.name === 'tabs' && args.action === 'new' && tabsNewPage) {
      return {
        isError: false,
        content: [{ type: 'text', text: 'opened' }],
        structuredContent: { page: tabsNewPage },
      }
    }
    if (
      tool.name === 'tab_groups' &&
      args.action === 'create' &&
      args.groupId === undefined &&
      pendingRootGroupCreate
    ) {
      return pendingRootGroupCreate
    }
    if (
      tool.name === 'tab_groups' &&
      args.action === 'close' &&
      pendingGroupClose
    ) {
      return pendingGroupClose
    }
    if (tool.name === 'tab_groups' && args.action === nextGroupErrorAction) {
      nextGroupErrorAction = null
      return {
        isError: true,
        content: [{ type: 'text', text: 'transient group failure' }],
      }
    }
    return { isError: false, content: [{ type: 'text', text: 'ok' }] }
  },
}))

const { ownershipStore } = await import('../../src/domain/ownership')
const { env } = await import('../../src/env')
const { setBrowserSession } = await import('../../src/lib/browser-session')
const { identityService } = await import('../../src/lib/mcp-session')
const {
  getSessionRefsForTesting,
  reapRetainedSessions,
  resetSingleMcpInstanceForTesting,
  setLastActivityForTesting,
  sweepIdleSessions,
} = await import('../../src/mcp/single-server')
const { getAuditDb, resetAuditDbForTesting, setAuditDbForTesting } =
  await import('../../src/modules/db/db')
const { dispatchCancellation } = await import(
  '../../src/services/dispatch-cancellation'
)
const {
  clearFirstCapturesForTesting,
  hasFirstCapturesForTesting,
  markFirstCaptureForTesting,
} = await import('../../src/services/screenshots')
const app = (await import('../../src/server')).default

async function connect(clientName = 'claude-code') {
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
  return { client, transport, sessionId, identity }
}

function endRowsFor(
  sessionId: string,
): Array<{ kind: string; reason: string | null }> {
  return getAuditDb()
    .select({ kind: agentSessionEnds.kind, reason: agentSessionEnds.reason })
    .from(agentSessionEnds)
    .where(eq(agentSessionEnds.sessionId, sessionId))
    .all()
}

function startRowsFor(sessionId: string): Array<{ agentId: string }> {
  return getAuditDb()
    .select({ agentId: agentSessionStarts.agentId })
    .from(agentSessionStarts)
    .where(eq(agentSessionStarts.sessionId, sessionId))
    .all()
}

function seedOwnership(
  key: ReturnType<typeof identityService.list>[number]['key'],
): void {
  ownershipStore.claimPage(key, 7)
  ownershipStore.claimPage(key, 8)
  ownershipStore.setGroup(key, {
    id: 'G1',
    windowId: 1,
    color: 'red',
    title: 'claude/invoice-processing',
    collapsed: false,
  })
  markFirstCaptureForTesting(key, 7)
}

const ORIGINAL_IDLE = env.sessionIdleMs
const ORIGINAL_RETENTION = env.sessionRetentionMs

describe('MCP session lifecycle', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    ownershipStore.clear()
    dispatchCancellation.clear()
    clearFirstCapturesForTesting()
    groupCalls.length = 0
    nextGroupErrorAction = null
    tabsNewPage = null
    pendingRootGroupCreate = null
    pendingGroupClose = null
    setBrowserSession(null)
    env.sessionIdleMs = 50
    env.sessionRetentionMs = 1_000
  })

  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    ownershipStore.clear()
    dispatchCancellation.clear()
    clearFirstCapturesForTesting()
    groupCalls.length = 0
    nextGroupErrorAction = null
    tabsNewPage = null
    pendingRootGroupCreate = null
    pendingGroupClose = null
    setBrowserSession(null)
    env.sessionIdleMs = ORIGINAL_IDLE
    env.sessionRetentionMs = ORIGINAL_RETENTION
    resetAuditDbForTesting()
  })

  test('uses 30 minute idle and 2 hour retention defaults', () => {
    expect(ORIGINAL_IDLE).toBe(30 * 60 * 1_000)
    expect(ORIGINAL_RETENTION).toBe(2 * 60 * 60 * 1_000)
  })

  test('duplicate initialized notifications keep one identity and start row', async () => {
    const { client, sessionId, identity } = await connect()
    const refs = getSessionRefsForTesting(sessionId)
    if (!refs) throw new Error('missing session refs')

    refs.server.server.oninitialized?.()

    expect(identityService.getIdentity(sessionId)).toBe(identity)
    expect(startRowsFor(sessionId)).toEqual([{ agentId: identity.key }])
    await client.close()
  })

  test('idle teardown collapses and retains ownership without closing', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)

    expect(sweepIdleSessions(Date.now())).toEqual([sessionId])
    await Bun.sleep(0)

    expect(identityService.getIdentity(sessionId)).toBeNull()
    expect(identityService.listRetained()).toHaveLength(1)
    expect(endRowsFor(sessionId)).toEqual([{ kind: 'closed', reason: null }])
    expect(groupCalls.map((call) => call.args)).toEqual([
      { action: 'update', groupId: 'G1', collapsed: true },
    ])
    expect(ownershipStore.ownerOf(7)).toBe(identity.key)
    expect(ownershipStore.groupOf(identity.key)?.collapsed).toBe(true)
    expect(hasFirstCapturesForTesting(identity.key)).toBe(true)
    expect(sweepIdleSessions(Date.now())).toEqual([])
    expect(endRowsFor(sessionId)).toHaveLength(1)
    await client.close()
  })

  test('explicit client DELETE follows the same collapse and retain path', async () => {
    const { client, transport, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)

    const response = await app.fetch(
      new Request('http://localhost/mcp', {
        method: 'DELETE',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          'mcp-session-id': transport.sessionId as string,
        },
      }),
    )
    expect(response.status).toBe(200)
    await Promise.resolve()

    expect(identityService.getIdentity(sessionId)).toBeNull()
    expect(identityService.listRetained()).toMatchObject([
      { key: identity.key },
    ])
    expect(groupCalls[0]?.args).toEqual({
      action: 'update',
      groupId: 'G1',
      collapsed: true,
    })
    await client.close()
  })

  test('DELETE waits for an in-flight first-group create before collapse', async () => {
    const { client, transport, identity } = await connect()
    setBrowserSession({
      pages: {
        getInfo: (pageId: number) => ({
          targetId: `target-${pageId}`,
          url: 'https://example.com',
          title: 'Example',
        }),
      },
    } as never)
    tabsNewPage = 9
    let releaseCreate:
      | ((result: Awaited<NonNullable<typeof pendingRootGroupCreate>>) => void)
      | undefined
    pendingRootGroupCreate = new Promise((resolve) => {
      releaseCreate = resolve
    })

    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://example.com' },
    })
    const response = await app.fetch(
      new Request('http://localhost/mcp', {
        method: 'DELETE',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          'mcp-session-id': transport.sessionId as string,
        },
      }),
    )
    expect(response.status).toBe(200)
    expect(identityService.listRetained()).toMatchObject([
      { key: identity.key },
    ])
    expect(ownershipStore.groupOf(identity.key)).toBeNull()

    releaseCreate?.({
      isError: false,
      content: [{ type: 'text', text: 'created' }],
      structuredContent: { group: { groupId: 'G1', windowId: 1 } },
    })
    await Bun.sleep(0)

    expect(groupCalls.map((call) => call.args)).toContainEqual({
      action: 'update',
      groupId: 'G1',
      collapsed: true,
    })
    expect(ownershipStore.groupOf(identity.key)?.collapsed).toBe(true)
    await client.close()
  })

  test('transport error records errored and tears down the session', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)
    const refs = getSessionRefsForTesting(sessionId)
    if (!refs) throw new Error('missing session refs')

    refs.transport.onerror?.(new Error('socket broke'))
    await Bun.sleep(0)

    expect(identityService.getIdentity(sessionId)).toBeNull()
    expect(endRowsFor(sessionId)).toEqual([
      { kind: 'errored', reason: 'socket broke' },
    ])
    expect(ownershipStore.groupOf(identity.key)?.collapsed).toBe(true)
    await client.close()
  })

  test('idle teardown closes transport and server and aborts dispatches', async () => {
    const { client, sessionId } = await connect()
    const refs = getSessionRefsForTesting(sessionId)
    if (!refs) throw new Error('missing session refs')
    let transportClosed = 0
    let serverClosed = 0
    refs.transport.close = async () => {
      transportClosed += 1
    }
    refs.server.close = async () => {
      serverClosed += 1
    }
    const controller = new AbortController()
    dispatchCancellation.register(sessionId, controller)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)

    expect(sweepIdleSessions(Date.now())).toEqual([sessionId])

    expect(transportClosed).toBe(1)
    expect(serverClosed).toBe(1)
    expect(controller.signal.aborted).toBe(true)
    expect(controller.signal.reason).toBe('MCP session ended')
    await client.close()
  })

  test('does not reap retained state before the retention boundary', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    const endedAt = identityService.listRetained()[0]?.endedAt
    if (endedAt === undefined) throw new Error('missing retained timestamp')

    expect(await reapRetainedSessions(endedAt + 999)).toEqual([])
    expect(ownershipStore.ownerOf(7)).toBe(identity.key)
    expect(identityService.listRetained()).toHaveLength(1)
    await client.close()
  })

  test('retries a failed collapse while the key is retained', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)
    nextGroupErrorAction = 'update'
    setLastActivityForTesting(sessionId, Date.now() - 10_000)

    sweepIdleSessions(Date.now())
    await Bun.sleep(0)
    expect(ownershipStore.groupOf(identity.key)?.collapsed).toBe(false)
    const endedAt = identityService.listRetained()[0]?.endedAt
    if (endedAt === undefined) throw new Error('missing retained timestamp')

    expect(await reapRetainedSessions(endedAt + 999)).toEqual([])
    expect(ownershipStore.groupOf(identity.key)?.collapsed).toBe(true)
    expect(
      groupCalls.filter((call) => call.args.action === 'update'),
    ).toHaveLength(2)
    await client.close()
  })

  test('expiry closes the group then forgets ownership and captures', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    await Promise.resolve()
    groupCalls.length = 0
    const endedAt = identityService.listRetained()[0]?.endedAt
    if (endedAt === undefined) throw new Error('missing retained timestamp')

    expect(await reapRetainedSessions(endedAt + 1_000)).toEqual([identity.key])

    expect(groupCalls.map((call) => call.args)).toEqual([
      { action: 'close', groupId: 'G1' },
    ])
    expect(groupCalls.some((call) => call.toolName === 'tabs')).toBe(false)
    expect(ownershipStore.ownerOf(7)).toBeNull()
    expect(ownershipStore.ownerOf(8)).toBeNull()
    expect(ownershipStore.groupOf(identity.key)).toBeNull()
    expect(hasFirstCapturesForTesting(identity.key)).toBe(false)
    expect(identityService.listRetained()).toEqual([])
    expect(await reapRetainedSessions(endedAt + 2_000)).toEqual([])
    await client.close()
  })

  test('connected close failure keeps retained state for a later retry', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    await Bun.sleep(0)
    groupCalls.length = 0
    const endedAt = identityService.listRetained()[0]?.endedAt
    if (endedAt === undefined) throw new Error('missing retained timestamp')
    nextGroupErrorAction = 'close'

    expect(await reapRetainedSessions(endedAt + 1_000)).toEqual([])
    expect(identityService.listRetained()).toHaveLength(1)
    expect(ownershipStore.ownerOf(7)).toBe(identity.key)
    expect(hasFirstCapturesForTesting(identity.key)).toBe(true)

    expect(await reapRetainedSessions(endedAt + 2_000)).toEqual([identity.key])
    expect(identityService.listRetained()).toEqual([])
    expect(ownershipStore.ownerOf(7)).toBeNull()
    expect(hasFirstCapturesForTesting(identity.key)).toBe(false)
    await client.close()
  })

  test('overlapping reaps issue only one group close', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setBrowserSession({} as never)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    await Bun.sleep(0)
    groupCalls.length = 0
    const endedAt = identityService.listRetained()[0]?.endedAt
    if (endedAt === undefined) throw new Error('missing retained timestamp')
    let releaseClose:
      | ((result: Awaited<NonNullable<typeof pendingGroupClose>>) => void)
      | undefined
    pendingGroupClose = new Promise((resolve) => {
      releaseClose = resolve
    })

    const first = reapRetainedSessions(endedAt + 1_000)
    await Bun.sleep(0)
    const second = reapRetainedSessions(endedAt + 1_000)
    await Bun.sleep(0)
    expect(
      groupCalls.filter((call) => call.args.action === 'close'),
    ).toHaveLength(1)

    releaseClose?.({
      isError: false,
      content: [{ type: 'text', text: 'closed' }],
    })
    expect(await first).toEqual([identity.key])
    expect(await second).toEqual([])
    expect(identityService.listRetained()).toEqual([])
    await client.close()
  })

  test('disconnected-browser expiry forgets state without CDP', async () => {
    const { client, sessionId, identity } = await connect()
    seedOwnership(identity.key)
    setLastActivityForTesting(sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    const endedAt = identityService.listRetained()[0]?.endedAt
    if (endedAt === undefined) throw new Error('missing retained timestamp')
    groupCalls.length = 0
    setBrowserSession(null)

    expect(await reapRetainedSessions(endedAt + 1_000)).toEqual([identity.key])
    expect(groupCalls).toEqual([])
    expect(ownershipStore.ownerOf(7)).toBeNull()
    expect(identityService.listRetained()).toEqual([])
    await client.close()
  })
})
