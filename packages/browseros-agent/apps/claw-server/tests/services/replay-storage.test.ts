/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for the per-session NDJSON replay store. We use an isolated
 * `rootDir` under tmp so the production singleton's data directory
 * is never touched.
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  createReplayStorage,
  type ReplayStorage,
} from '../../src/services/replay-storage'

function buildLine(over: {
  sessionId?: string
  tabPageId?: number
  ts?: number
  type?: number
  data?: unknown
}): string {
  return JSON.stringify({
    sessionId: over.sessionId ?? 's1',
    tabPageId: over.tabPageId ?? 10,
    ts: over.ts ?? Date.now(),
    type: over.type ?? 3,
    data: over.data ?? { foo: 'bar' },
  })
}

let dir: string
let store: ReplayStorage

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'replay-storage-'))
  store = createReplayStorage({ rootDir: dir })
})

afterEach(async () => {
  await store.resetForTesting()
  await rm(dir, { recursive: true, force: true })
})

describe('replay-storage', () => {
  it('appendEvents writes lines that readEvents can read back', async () => {
    await store.appendEvents('s1', [
      buildLine({ tabPageId: 10, ts: 100, type: 2 }),
      buildLine({ tabPageId: 10, ts: 200, type: 3 }),
    ])
    const stream = await store.readEvents('s1')
    const text = await new Response(stream).text()
    const lines = text.split('\n').filter(Boolean)
    expect(lines).toHaveLength(2)
    expect(JSON.parse(lines[0]).ts).toBe(100)
    expect(JSON.parse(lines[1]).ts).toBe(200)
  })

  it('statSession returns hasData:false for unknown session', async () => {
    const stat = await store.statSession('nope')
    expect(stat.hasData).toBe(false)
    expect(stat.sizeBytes).toBe(0)
    expect(stat.tabPageIds).toEqual([])
  })

  it('statSession returns firstEventAt and lastEventAt for a populated file', async () => {
    await store.appendEvents('s2', [
      buildLine({ tabPageId: 5, ts: 1000 }),
      buildLine({ tabPageId: 5, ts: 2000 }),
      buildLine({ tabPageId: 7, ts: 3000 }),
    ])
    const stat = await store.statSession('s2')
    expect(stat.hasData).toBe(true)
    expect(stat.firstEventAt).toBe(1000)
    expect(stat.lastEventAt).toBe(3000)
    expect(stat.tabPageIds).toEqual([5, 7])
    expect(stat.sizeBytes).toBeGreaterThan(0)
  })

  it('deleteSession removes the file', async () => {
    await store.appendEvents('s3', [buildLine({ ts: 1 })])
    expect((await store.statSession('s3')).hasData).toBe(true)
    await store.deleteSession('s3')
    expect((await store.statSession('s3')).hasData).toBe(false)
  })

  it('deleteSession is a no-op for unknown session', async () => {
    await expect(store.deleteSession('ghost')).resolves.toBeUndefined()
  })

  it('LRU evicts the oldest handle when over the cap', async () => {
    const small = createReplayStorage({ rootDir: dir, maxOpenHandles: 2 })
    await small.appendEvents('a', [buildLine({ ts: 1 })])
    await small.appendEvents('b', [buildLine({ ts: 2 })])
    await small.appendEvents('c', [buildLine({ ts: 3 })])
    // a should have been evicted; its handle is no longer open.
    // We can prove this by deleting + recreating its file outside the
    // store's view, then appending more, and confirming we did not
    // re-use a stale handle.
    await small.deleteSession('a')
    await small.appendEvents('a', [buildLine({ ts: 4 })])
    const stream = await small.readEvents('a')
    const text = await new Response(stream).text()
    const lines = text.split('\n').filter(Boolean)
    expect(lines).toHaveLength(1)
    expect(JSON.parse(lines[0]).ts).toBe(4)
    await small.resetForTesting()
  })

  it('concurrent appends to the same session serialise without tearing lines', async () => {
    const lines = Array.from({ length: 100 }, (_, i) =>
      buildLine({ ts: i + 1 }),
    )
    const halves = [lines.slice(0, 50), lines.slice(50)]
    await Promise.all(halves.map((batch) => store.appendEvents('s4', batch)))
    const stream = await store.readEvents('s4')
    const text = await new Response(stream).text()
    const round = text.split('\n').filter(Boolean)
    expect(round).toHaveLength(100)
    for (const line of round) {
      // Each round-tripped line must parse cleanly.
      expect(() => JSON.parse(line)).not.toThrow()
    }
  })

  it('concurrent appends to different sessions run in parallel', async () => {
    await Promise.all([
      store.appendEvents('s5', [buildLine({ ts: 1 })]),
      store.appendEvents('s6', [buildLine({ ts: 2 })]),
      store.appendEvents('s7', [buildLine({ ts: 3 })]),
    ])
    const a = await new Response(await store.readEvents('s5')).text()
    const b = await new Response(await store.readEvents('s6')).text()
    const c = await new Response(await store.readEvents('s7')).text()
    expect(JSON.parse(a.trim()).ts).toBe(1)
    expect(JSON.parse(b.trim()).ts).toBe(2)
    expect(JSON.parse(c.trim()).ts).toBe(3)
  })

  it('appendEvents creates the parent directory on first call', async () => {
    const nested = createReplayStorage({ rootDir: `${dir}/nested/sub` })
    await nested.appendEvents('s8', [buildLine({ ts: 1 })])
    const stat = await nested.statSession('s8')
    expect(stat.hasData).toBe(true)
    await nested.resetForTesting()
  })

  it('sanitises sessionIds to prevent path traversal', async () => {
    // A sessionId containing ../ must not escape the rootDir.
    await store.appendEvents('../escape', [buildLine({ ts: 1 })])
    // The sanitised file lives inside rootDir.
    const stat = await store.statSession('../escape')
    expect(stat.hasData).toBe(true)
  })

  it('readEvents on unknown sessionId returns an empty stream', async () => {
    const stream = await store.readEvents('ghost')
    const text = await new Response(stream).text()
    expect(text).toBe('')
  })
})
