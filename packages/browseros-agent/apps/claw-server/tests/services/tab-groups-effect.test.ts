/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import { agentKeyFromSlug } from '../../src/domain/agent-key'
import { ownershipStore } from '../../src/domain/ownership'
import type { ClientIdentity } from '../../src/lib/mcp-session'

interface FakeResult {
  isError: boolean
  content: Array<{ type: 'text'; text: string }>
  structuredContent?: unknown
}

interface CallEntry {
  args: Record<string, unknown>
  signal?: AbortSignal
}

const calls: CallEntry[] = []
const queued: Array<FakeResult | Error | Promise<FakeResult>> = []

const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (
    _tool: unknown,
    args: Record<string, unknown>,
    context: { signal?: AbortSignal },
  ) => {
    calls.push({ args, signal: context.signal })
    const result = queued.shift()
    if (!result) throw new Error('tab-groups-effect: no queued result')
    if (result instanceof Error) throw result
    return result
  },
}))

const {
  applyAgentTabGroupTitle,
  applyTabGroups,
  closeAgentTabGroup,
  collapseAgentTabGroup,
  ensureAgentTabGroup,
  resetTabGroupEffectsForTesting,
} = await import('../../src/mcp/effects/tab-groups')

const tabsTool = BROWSER_TOOLS.find((tool) => tool.name === 'tabs')
if (!tabsTool) throw new Error('tabs tool missing')

const fakeSession = {} as never
const key = agentKeyFromSlug('claude-code-swift-otter')

function identity(label = 'swift-otter'): ClientIdentity {
  return {
    sessionId: 'sid-1',
    clientName: 'claude-code',
    clientVersion: '1.0.0',
    clientTitle: null,
    slug: 'claude-code',
    key,
    generatedLabel: 'swift-otter',
    label,
    firstSeenAt: 1,
  }
}

function ok(structuredContent?: unknown): FakeResult {
  return {
    isError: false,
    content: [{ type: 'text', text: 'ok' }],
    structuredContent,
  }
}

function err(text: string): FakeResult {
  return { isError: true, content: [{ type: 'text', text }] }
}

function queue(
  ...results: Array<FakeResult | Error | Promise<FakeResult>>
): void {
  queued.push(...results)
}

function setGroup(collapsed = false): void {
  ownershipStore.setGroup(key, {
    id: 'G1',
    windowId: 1,
    color: 'red',
    title: 'claude/swift-otter',
    collapsed,
  })
}

function sessionWithPageGroup(groupId?: string): never {
  return {
    pages: {
      getInfo: () => ({ groupId }),
    },
  } as never
}

beforeEach(() => {
  calls.length = 0
  queued.length = 0
  ownershipStore.clear()
  resetTabGroupEffectsForTesting()
})

afterEach(() => {
  queued.length = 0
  ownershipStore.clear()
  resetTabGroupEffectsForTesting()
})

describe('durable tab group effect', () => {
  it('single-flights racing creates and adds the waiting page afterward', async () => {
    let release: ((result: FakeResult) => void) | undefined
    const createResult = new Promise<FakeResult>((resolve) => {
      release = resolve
    })
    queue(createResult, ok(), ok())

    const first = ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 1,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    const second = ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 2,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    release?.(ok({ group: { groupId: 'G1', windowId: 1 } }))
    await Promise.all([first, second])

    const rootCreates = calls.filter(
      (call) =>
        call.args.action === 'create' && call.args.groupId === undefined,
    )
    expect(rootCreates).toHaveLength(1)
    expect(calls.at(-1)?.args).toEqual({
      action: 'create',
      groupId: 'G1',
      pages: [2],
    })
  })

  it('shares a caught create failure across racing awaiters', async () => {
    let release: ((result: FakeResult) => void) | undefined
    const createResult = new Promise<FakeResult>((resolve) => {
      release = resolve
    })
    queue(createResult)
    const inputs = [1, 2].map((pageId) =>
      ensureAgentTabGroup({
        key,
        identity: identity(),
        pageId,
        session: fakeSession,
        defaultTabGroupId: null,
      }),
    )
    release?.(err('manager down'))

    await expect(Promise.all(inputs)).resolves.toEqual([undefined, undefined])
    expect(calls).toHaveLength(1)
    expect(ownershipStore.groupOf(key)).toBeNull()
  })

  it('waits for an in-flight create before collapsing the new group', async () => {
    let release: ((result: FakeResult) => void) | undefined
    const createResult = new Promise<FakeResult>((resolve) => {
      release = resolve
    })
    queue(createResult, ok(), ok())

    const creating = ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 1,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    const collapsing = collapseAgentTabGroup({ key, session: fakeSession })
    expect(calls).toHaveLength(1)

    release?.(ok({ group: { groupId: 'G1', windowId: 1 } }))
    await Promise.all([creating, collapsing])

    expect(calls.map((call) => call.args)).toEqual([
      {
        action: 'create',
        pages: [1],
        title: 'claude/swift-otter',
      },
      { action: 'update', groupId: 'G1', color: 'red' },
      { action: 'update', groupId: 'G1', collapsed: true },
    ])
    expect(ownershipStore.groupOf(key)?.collapsed).toBe(true)
  })

  it('uses its own timeout signal instead of the aborted client signal', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 1 } }), ok())
    const client = new AbortController()
    client.abort()

    applyTabGroups({
      call: {
        tool: tabsTool,
        args: { action: 'new' },
        sessionId: 'sid-1',
        identity: identity(),
        key,
        agent: { agentId: key, slug: 'claude-code' },
        agentLabel: 'claude-code',
        session: fakeSession,
        signal: client.signal,
        defaultTabGroupId: null,
        flags: { newPage: true, closePage: false, listTabs: false },
      },
      result: {
        content: [{ type: 'text', text: 'ok' }],
        structuredContent: { page: 1 },
      },
      cancelled: false,
      durationMs: 1,
    })
    await Promise.resolve()
    await Promise.resolve()

    expect(calls[0]?.signal).toBeInstanceOf(AbortSignal)
    expect(calls[0]?.signal).not.toBe(client.signal)
    expect(calls[0]?.signal?.aborted).toBe(false)
  })

  it('clears the group on any tool-reported add failure and recreates it', async () => {
    setGroup()
    queue(err('opaque BrowserOS CDP failure'))
    await ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 2,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    expect(ownershipStore.groupOf(key)).toBeNull()

    queue(ok({ group: { groupId: 'G2', windowId: 1 } }), ok())
    await ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 3,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    expect(ownershipStore.groupOf(key)?.id).toBe('G2')
  })

  it('keeps the group on a thrown timeout and does not recreate it', async () => {
    setGroup()
    queue(new DOMException('operation timed out', 'TimeoutError'))
    await ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 2,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    expect(ownershipStore.groupOf(key)?.id).toBe('G1')

    calls.length = 0
    await ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 3,
      session: fakeSession,
      defaultTabGroupId: 'G1',
    })
    expect(calls).toHaveLength(0)
    expect(ownershipStore.groupOf(key)?.id).toBe('G1')
  })

  it('keeps a verified default group without dispatching group ops', async () => {
    setGroup()
    applyTabGroups({
      call: {
        tool: tabsTool,
        args: { action: 'new' },
        sessionId: 'sid-1',
        identity: identity(),
        key,
        agent: { agentId: key, slug: 'claude-code' },
        agentLabel: 'claude-code',
        session: sessionWithPageGroup('G1'),
        defaultTabGroupId: 'G1',
        flags: { newPage: true, closePage: false, listTabs: false },
      },
      result: {
        content: [{ type: 'text', text: 'ok' }],
        structuredContent: { page: 2 },
      },
      cancelled: false,
      durationMs: 1,
    })
    await Promise.resolve()

    expect(calls).toHaveLength(0)
    expect(ownershipStore.groupOf(key)?.id).toBe('G1')
  })

  it('recreates the group when the new page missed the default group', async () => {
    setGroup()
    queue(ok({ group: { groupId: 'G2', windowId: 1 } }), ok())
    applyTabGroups({
      call: {
        tool: tabsTool,
        args: { action: 'new' },
        sessionId: 'sid-1',
        identity: identity(),
        key,
        agent: { agentId: key, slug: 'claude-code' },
        agentLabel: 'claude-code',
        session: sessionWithPageGroup('deleted-group'),
        defaultTabGroupId: 'G1',
        flags: { newPage: true, closePage: false, listTabs: false },
      },
      result: {
        content: [{ type: 'text', text: 'ok' }],
        structuredContent: { page: 2 },
      },
      cancelled: false,
      durationMs: 1,
    })
    await Promise.resolve()
    await Promise.resolve()

    expect(calls[0]?.args).toEqual({
      action: 'create',
      pages: [2],
      title: 'claude/swift-otter',
    })
    expect(ownershipStore.groupOf(key)?.id).toBe('G2')
  })

  it('keeps a created group when the color lock fails', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 1 } }), err('color rejected'))
    await ensureAgentTabGroup({
      key,
      identity: identity(),
      pageId: 1,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    expect(ownershipStore.groupOf(key)?.id).toBe('G1')
  })

  it('applies a renamed title with the group timeout and durable state', async () => {
    setGroup()
    queue(ok())
    await applyAgentTabGroupTitle({
      key,
      title: 'claude/invoice-processing',
      session: fakeSession,
    })

    expect(calls[0]?.args).toEqual({
      action: 'update',
      groupId: 'G1',
      title: 'claude/invoice-processing',
    })
    expect(calls[0]?.signal).toBeInstanceOf(AbortSignal)
    expect(ownershipStore.groupOf(key)?.title).toBe('claude/invoice-processing')
  })

  it('keeps the title captured when group creation begins', async () => {
    let release: ((result: FakeResult) => void) | undefined
    const createResult = new Promise<FakeResult>((resolve) => {
      release = resolve
    })
    const sessionIdentity = identity()
    queue(createResult, ok())

    const creating = ensureAgentTabGroup({
      key,
      identity: sessionIdentity,
      pageId: 1,
      session: fakeSession,
      defaultTabGroupId: null,
    })
    sessionIdentity.label = 'invoice-processing'
    release?.(ok({ group: { groupId: 'G1', windowId: 1 } }))
    await creating

    expect(calls).toHaveLength(2)
    expect(calls[0]?.args).toEqual({
      action: 'create',
      pages: [1],
      title: 'claude/swift-otter',
    })
    expect(ownershipStore.groupOf(key)).toMatchObject({
      title: 'claude/swift-otter',
      color: 'red',
    })
  })

  it('expands a collapsed group after any successful dispatch', async () => {
    setGroup(true)
    queue(ok())
    applyTabGroups({
      call: {
        tool: tabsTool,
        args: { action: 'list' },
        sessionId: 'sid-1',
        identity: identity(),
        key,
        agent: { agentId: key, slug: 'claude-code' },
        agentLabel: 'claude-code',
        session: fakeSession,
        defaultTabGroupId: 'G1',
        flags: { newPage: false, closePage: false, listTabs: true },
      },
      result: { content: [{ type: 'text', text: 'ok' }] },
      cancelled: false,
      durationMs: 1,
    })
    await Promise.resolve()
    await Promise.resolve()

    expect(calls[0]?.args).toEqual({
      action: 'update',
      groupId: 'G1',
      collapsed: false,
    })
    expect(ownershipStore.groupOf(key)?.collapsed).toBe(false)
  })

  it('keeps the group ref when an expand update fails', async () => {
    setGroup(true)
    queue(err('opaque BrowserOS CDP failure'))
    applyTabGroups({
      call: {
        tool: tabsTool,
        args: { action: 'list' },
        sessionId: 'sid-1',
        identity: identity(),
        key,
        agent: { agentId: key, slug: 'claude-code' },
        agentLabel: 'claude-code',
        session: fakeSession,
        defaultTabGroupId: 'G1',
        flags: { newPage: false, closePage: false, listTabs: true },
      },
      result: { content: [{ type: 'text', text: 'ok' }] },
      cancelled: false,
      durationMs: 1,
    })
    await Promise.resolve()
    await Promise.resolve()

    expect(ownershipStore.groupOf(key)?.id).toBe('G1')
    expect(ownershipStore.groupOf(key)?.collapsed).toBe(true)
  })

  it('keeps the group ref when a rename update fails', async () => {
    setGroup()
    queue(err('opaque BrowserOS CDP failure'))

    await applyAgentTabGroupTitle({
      key,
      title: 'claude/renamed',
      session: fakeSession,
    })

    expect(ownershipStore.groupOf(key)?.id).toBe('G1')
  })

  it('collapses a live group with the group timeout', async () => {
    setGroup()
    queue(ok())

    expect(await collapseAgentTabGroup({ key, session: fakeSession })).toBe(
      true,
    )

    expect(calls[0]?.args).toEqual({
      action: 'update',
      groupId: 'G1',
      collapsed: true,
    })
    expect(calls[0]?.signal).toBeInstanceOf(AbortSignal)
    expect(ownershipStore.groupOf(key)?.collapsed).toBe(true)
  })

  it('closes a live group and all of its current members', async () => {
    setGroup()
    queue(ok())

    expect(await closeAgentTabGroup({ key, session: fakeSession })).toBe(true)

    expect(calls[0]?.args).toEqual({ action: 'close', groupId: 'G1' })
    expect(calls[0]?.signal).toBeInstanceOf(AbortSignal)
  })

  it('treats a close error as success when the group is already gone', async () => {
    setGroup()
    queue(err('Tab group not found'), ok({ groups: [], count: 0 }))

    expect(await closeAgentTabGroup({ key, session: fakeSession })).toBe(true)

    expect(calls.map((call) => call.args)).toEqual([
      { action: 'close', groupId: 'G1' },
      { action: 'list' },
    ])
  })

  it('keeps a close failure retryable when group absence is unknown', async () => {
    setGroup()
    queue(err('transient close failure'), err('transient list failure'))

    expect(await closeAgentTabGroup({ key, session: fakeSession })).toBe(false)
  })
})
