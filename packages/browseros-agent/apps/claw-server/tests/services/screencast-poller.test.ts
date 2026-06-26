/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for the screencast poller's tick logic. We deliberately do
 * not start the setInterval here; instead we exercise the tick path
 * by mocking executeTool + the tab-activity registry and calling the
 * poller with a 1-shot interval that we immediately stop.
 *
 * mock.module persists across files in the same `bun test` run, so we
 * scope the mocks to known concerns (executeTool, tab-activity) and
 * let the real screencast-cache singleton drive the assertions.
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import * as frameworkReal from '@browseros/browser-mcp/tools/framework'
import type { TabActivityRecord } from '../../src/lib/tab-activity'
import { screencastCache } from '../../src/services/screencast-cache'

interface FakeResult {
  isError: boolean
  content: { type: string; text: string }[]
  structuredContent?: unknown
}

const calls: { toolName: string; args: Record<string, unknown> }[] = []
const queued: Map<number, FakeResult[]> = new Map()
let snapshotRecords: TabActivityRecord[] = []

function setSnapshot(records: TabActivityRecord[]): void {
  snapshotRecords = records
}

function queueFor(pageId: number, ...rs: FakeResult[]): void {
  const arr = queued.get(pageId) ?? []
  arr.push(...rs)
  queued.set(pageId, arr)
}

function ok(image: string): FakeResult {
  return {
    isError: false,
    content: [{ type: 'text', text: 'ok' }],
    structuredContent: { image },
  }
}

function badResult(): FakeResult {
  return {
    isError: true,
    content: [{ type: 'text', text: 'fail' }],
  }
}

mock.module('@browseros/browser-mcp/tools/framework', () => ({
  // Spread the real module so any sibling exports (e.g. textResult)
  // that other transitively-loaded modules import still resolve.
  ...frameworkReal,
  executeTool: async (def: { name: string }, args: Record<string, unknown>) => {
    calls.push({ toolName: def.name, args })
    const pageId = args.page as number
    const arr = queued.get(pageId)
    const result = arr?.shift()
    if (!result) {
      throw new Error(`no queued result for page=${pageId}`)
    }
    return result
  },
}))

const { startScreencastPoller } = await import(
  '../../src/services/screencast-poller'
)

// Tests inject this stub registry via opts.registry so we never
// have to mock the tab-activity module (mock.module leaks across
// files in the same bun-test run).
const stubRegistry = {
  snapshot: () => snapshotRecords,
  recordTool: () => undefined,
  size: () => snapshotRecords.length,
  clear: () => undefined,
}

function rec(
  pageId: number,
  status: 'active' | 'idle' = 'active',
  lastToolAt = 1_000_000,
): TabActivityRecord {
  return {
    targetId: `t-${pageId}`,
    pageId,
    url: `https://e.com/${pageId}`,
    title: `Page ${pageId}`,
    agentId: 'claude-code',
    slug: 'claude-code',
    firstToolAt: lastToolAt - 1000,
    lastToolAt,
    lastToolName: 'snapshot',
    toolCount: 1,
    recentTools: [],
    status,
  }
}

const fakeSession = {} as never

// Helper: start the poller, wait for the immediate first tick to
// complete (Promise microtask flush + small await), stop, and return.
async function runOneTick(): Promise<void> {
  const handle = startScreencastPoller({
    session: fakeSession,
    intervalMs: 60_000,
    registry: stubRegistry,
  })
  // Yield to the event loop so the immediate `void tick()` resolves.
  await new Promise((r) => setTimeout(r, 0))
  await new Promise((r) => setTimeout(r, 0))
  handle.stop()
}

describe('screencast poller', () => {
  beforeEach(() => {
    calls.length = 0
    queued.clear()
    snapshotRecords = []
    screencastCache.resetForTesting()
  })

  afterEach(() => {
    screencastCache.resetForTesting()
  })

  it('fans out to active tabs only', async () => {
    setSnapshot([rec(1, 'active'), rec(2, 'idle'), rec(3, 'active')])
    queueFor(1, ok('IMG1'))
    queueFor(3, ok('IMG3'))

    await runOneTick()

    expect(calls.map((c) => c.args.page).sort()).toEqual([1, 3])
    expect(screencastCache.get(1)?.jpegBase64).toBe('IMG1')
    expect(screencastCache.get(3)?.jpegBase64).toBe('IMG3')
    expect(screencastCache.get(2)).toBeNull()
  })

  it('writes the JPEG bytes + capturedAt on success', async () => {
    setSnapshot([rec(7)])
    queueFor(7, ok('AAA'))

    await runOneTick()

    const frame = screencastCache.get(7)
    expect(frame?.jpegBase64).toBe('AAA')
    expect(frame?.capturedAt).toBeGreaterThan(0)
    expect(frame?.byteLength).toBeGreaterThan(0)
  })

  it('marks failure when executeTool returns isError', async () => {
    setSnapshot([rec(5)])
    queueFor(5, badResult())

    await runOneTick()

    expect(screencastCache.get(5)).toBeNull()
    // One failure is not enough to enter backoff with the default cap.
    expect(screencastCache.isInBackoff(5, 0)).toBe(false)
  })

  it('three consecutive failures put the page in backoff', async () => {
    setSnapshot([rec(9, 'active', 1_000)])
    queueFor(9, badResult(), badResult(), badResult())

    await runOneTick()
    await runOneTick()
    await runOneTick()

    // sinceMs from the deep past keeps us in backoff.
    expect(screencastCache.isInBackoff(9, 1_000)).toBe(true)
    // A new agent dispatch (advances lastToolAt past the recorded
    // lastFailureAt) lifts the backoff. markFailure() stamps
    // Date.now(), so use a comfortably-future value.
    expect(screencastCache.isInBackoff(9, Date.now() + 60_000)).toBe(false)
  })

  it('does not call executeTool for a page already in backoff with stale lastToolAt', async () => {
    setSnapshot([rec(4, 'active', 1_000)])
    queueFor(4, badResult(), badResult(), badResult())

    await runOneTick()
    await runOneTick()
    await runOneTick()
    expect(calls.map((c) => c.args.page)).toEqual([4, 4, 4])

    // Next tick with same lastToolAt: still in backoff; expected
    // no new calls. We queue nothing because no call should fire.
    calls.length = 0
    await runOneTick()
    expect(calls).toEqual([])
  })

  it('successful capture clears the failure counter', async () => {
    setSnapshot([rec(8, 'active', 1_000)])
    queueFor(8, badResult(), badResult(), ok('FRESH'))

    await runOneTick()
    await runOneTick()
    await runOneTick()

    expect(screencastCache.get(8)?.jpegBase64).toBe('FRESH')
    expect(screencastCache.isInBackoff(8, 1_000)).toBe(false)
  })

  it('GCs frames for pageIds no longer in the registry', async () => {
    setSnapshot([rec(11, 'active')])
    queueFor(11, ok('LIVE'))
    await runOneTick()
    expect(screencastCache.get(11)).not.toBeNull()

    // Tab closed; registry no longer reports pageId 11.
    setSnapshot([])
    await runOneTick()
    expect(screencastCache.get(11)).toBeNull()
  })

  it('does not throw when one page fails and another succeeds', async () => {
    setSnapshot([rec(21), rec(22)])
    queueFor(21, badResult())
    queueFor(22, ok('OK22'))

    await runOneTick()

    expect(screencastCache.get(21)).toBeNull()
    expect(screencastCache.get(22)?.jpegBase64).toBe('OK22')
  })

  it('keeps the prior frame on a single transient failure', async () => {
    setSnapshot([rec(31, 'active', 1_000)])
    queueFor(31, ok('GOOD'), badResult())

    await runOneTick() // populates cache with GOOD
    expect(screencastCache.get(31)?.jpegBase64).toBe('GOOD')

    await runOneTick() // one failure; not yet in backoff
    expect(screencastCache.get(31)?.jpegBase64).toBe('GOOD')
  })

  it('drops the stale frame once the page enters backoff', async () => {
    setSnapshot([rec(32, 'active', 1_000)])
    queueFor(32, ok('OLD'), badResult(), badResult(), badResult())

    await runOneTick() // OLD frame in cache
    expect(screencastCache.get(32)?.jpegBase64).toBe('OLD')

    await runOneTick() // failure 1: keep frame
    expect(screencastCache.get(32)?.jpegBase64).toBe('OLD')
    await runOneTick() // failure 2: keep frame
    expect(screencastCache.get(32)?.jpegBase64).toBe('OLD')
    await runOneTick() // failure 3: enter backoff -> drop the stale frame
    expect(screencastCache.get(32)).toBeNull()
  })
})
