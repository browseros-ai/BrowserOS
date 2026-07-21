/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { recordToolDispatch } from '../../src/services/audit-log'
import { legacyScreenshotPath } from '../../src/services/screenshots'
import {
  recordSessionEnd,
  recordSessionStart,
} from '../../src/services/session-events'
import { getTask, getTaskSummaries, listTasks } from '../../src/services/tasks'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

/**
 * Seeds a legacy flat file to verify old captures still appear through
 * session-owned task metadata.
 */
function seedScreenshotFile(dispatchId: number | null | undefined): void {
  if (typeof dispatchId !== 'number') return
  const path = legacyScreenshotPath(dispatchId)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, Buffer.from([0xff, 0xd8, 0xff, 0xd9]))
}

function dispatch(
  sessionId: string,
  toolName: string,
  opts: { url?: string; isError?: boolean } = {},
) {
  return recordToolDispatch({
    agentId: sessionId.startsWith('cc-') ? 'claude-code' : 'cursor',
    slug: sessionId.startsWith('cc-') ? 'claude-code' : 'cursor',
    agentLabel: sessionId.startsWith('cc-') ? 'Claude Code' : 'Cursor',
    sessionId,
    toolName,
    pageId: 1,
    targetId: null,
    url: opts.url ?? null,
    title: null,
    rawArgs: {},
    durationMs: 5,
    result: {
      isError: opts.isError ?? false,
      structuredContent: {},
      content: [{ type: 'image', data: 'AAA', mimeType: 'image/jpeg' }],
    },
  })
}

function startSession(sessionId: string) {
  recordSessionStart({
    sessionId,
    agentId: sessionId.startsWith('cc-') ? 'claude-code' : 'cursor',
    slug: sessionId.startsWith('cc-') ? 'claude-code' : 'cursor',
    agentLabel: sessionId.startsWith('cc-') ? 'Claude Code' : 'Cursor',
    clientName: sessionId.startsWith('cc-') ? 'claude-code' : 'cursor',
    clientVersion: '0.0.0',
  })
}

describe('listTasks', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('returns one task per session and counts dispatches', () => {
    startSession('cc-a')
    dispatch('cc-a', 'tabs', { url: 'https://news.example.com' })
    dispatch('cc-a', 'snapshot')
    dispatch('cc-a', 'read')

    startSession('cur-b')
    dispatch('cur-b', 'tabs', { url: 'https://docs.example.com' })

    const r = listTasks({})
    expect(r.tasks).toHaveLength(2)
    const a = r.tasks.find((t) => t.sessionId === 'cc-a')!
    expect(a.dispatchCount).toBe(3)
    expect(a.toolSequence).toEqual(['tabs', 'snapshot', 'read'])
    expect(a.title).toBe('Browsed news.example.com')
    expect(a.site).toBe('news.example.com')
    expect(a.agentLabel).toBe('Claude Code')
  })

  it('derives status: failed when any dispatch isError=true', () => {
    dispatch('cc-x', 'tabs', { url: 'https://e.com' })
    dispatch('cc-x', 'act', { isError: true })
    const r = listTasks({})
    expect(r.tasks[0]!.status).toBe('failed')
    expect(r.tasks[0]!.errorCount).toBe(1)
  })

  it('derives status: done when an end row is present', () => {
    dispatch('cc-y', 'tabs', { url: 'https://e.com' })
    recordSessionEnd({ sessionId: 'cc-y', kind: 'closed' })
    expect(listTasks({}).tasks[0]!.status).toBe('done')
  })

  it('derives status: failed when end row kind=errored', () => {
    dispatch('cc-z', 'tabs', { url: 'https://e.com' })
    recordSessionEnd({ sessionId: 'cc-z', kind: 'errored', reason: 'oops' })
    expect(listTasks({}).tasks[0]!.status).toBe('failed')
  })

  it('derives status: live when no end row and recent dispatch', () => {
    dispatch('cc-live', 'tabs', { url: 'https://e.com' })
    expect(listTasks({}).tasks[0]!.status).toBe('live')
  })

  it('applies agentId filter at the SQL layer', () => {
    dispatch('cc-1', 'tabs', { url: 'https://e.com' })
    dispatch('cur-1', 'tabs', { url: 'https://e.com' })
    const r = listTasks({ agentId: 'claude-code' })
    expect(r.tasks).toHaveLength(1)
    expect(r.tasks[0]!.sessionId).toBe('cc-1')
  })

  it('applies status filter in JS', () => {
    dispatch('cc-ok', 'tabs', { url: 'https://e.com' })
    recordSessionEnd({ sessionId: 'cc-ok', kind: 'closed' })
    dispatch('cc-bad', 'tabs', { url: 'https://e.com' })
    dispatch('cc-bad', 'act', { isError: true })
    const r = listTasks({ status: 'done' })
    expect(r.tasks).toHaveLength(1)
    expect(r.tasks[0]!.sessionId).toBe('cc-ok')
  })

  it('paginates via cursor', () => {
    for (let i = 0; i < 6; i++) {
      dispatch(`cc-p${i}`, 'tabs', { url: 'https://e.com' })
    }
    const page1 = listTasks({ limit: 3 })
    expect(page1.tasks).toHaveLength(3)
    expect(page1.nextCursor).not.toBeNull()
    const page2 = listTasks({ limit: 3, cursor: page1.nextCursor! })
    expect(page2.tasks).toHaveLength(3)
    expect(page2.nextCursor).toBeNull()
    const all = new Set([
      ...page1.tasks.map((t) => t.sessionId),
      ...page2.tasks.map((t) => t.sessionId),
    ])
    expect(all.size).toBe(6)
  })

  it('does not duplicate or truncate sessions when dispatches interleave across the cursor', () => {
    // Two busy sessions whose dispatch ids straddle the page cursor, mixed
    // with short sessions. Ids are assigned in call order:
    dispatch('cc-s1', 'a') // id 1
    dispatch('cc-s2', 'a') // id 2
    dispatch('cc-s3', 'a') // id 3
    dispatch('cc-s4', 'a') // id 4
    dispatch('cc-s1', 'b') // id 5  -> cc-s1 = {1, 5}
    dispatch('cc-s2', 'b') // id 6  -> cc-s2 = {2, 6}

    const seen = new Map<string, number>()
    let cursor: number | undefined
    for (let guard = 0; guard < 10; guard++) {
      const page = listTasks({ limit: 2, cursor })
      for (const t of page.tasks) {
        // Each session must appear on exactly one page.
        expect(seen.has(t.sessionId)).toBe(false)
        seen.set(t.sessionId, t.dispatchCount)
      }
      if (page.nextCursor == null) break
      cursor = page.nextCursor
    }

    expect(seen.size).toBe(4)
    // Counts must reflect all rows, not just those below the cursor.
    expect(seen.get('cc-s1')).toBe(2)
    expect(seen.get('cc-s2')).toBe(2)
  })
})

describe('getTask', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('returns null for unknown session', () => {
    expect(getTask('nope')).toBeNull()
  })

  it('returns dispatches + screenshot ids in chronological order', async () => {
    await withTempBrowserClawDir(async () => {
      startSession('cc-screens')
      dispatch('cc-screens', 'tabs', { url: 'https://e.com' })
      const id1 = dispatch('cc-screens', 'screenshot')
      dispatch('cc-screens', 'read')
      const id2 = dispatch('cc-screens', 'screenshot')
      recordSessionEnd({ sessionId: 'cc-screens', kind: 'closed' })
      // Simulate that persistScreenshot wrote a file for both.
      seedScreenshotFile(id1)
      seedScreenshotFile(id2)

      const detail = getTask('cc-screens')!
      expect(detail.dispatches).toHaveLength(4)
      expect(detail.screenshotIds).toEqual([id1!, id2!])
      expect(detail.startEvent?.clientName).toBe('claude-code')
      expect(detail.endEvent?.kind).toBe('closed')
      expect(detail.status).toBe('done')
    })
  })

  it('includes error-result screenshots in session history', async () => {
    await withTempBrowserClawDir(async () => {
      dispatch('cc-err', 'tabs', { url: 'https://e.com' })
      const ok = dispatch('cc-err', 'screenshot')
      const bad = dispatch('cc-err', 'screenshot', { isError: true })
      seedScreenshotFile(ok)
      seedScreenshotFile(bad)
      const detail = getTask('cc-err')!
      expect(detail.screenshotIds).toEqual([ok!, bad!])
    })
  })

  it('includes screenshots for any audited tool whose JPEG exists', async () => {
    await withTempBrowserClawDir(async () => {
      const nav = dispatch('cc-fb', 'navigate', { url: 'https://e.com' })
      const first_read = dispatch('cc-fb', 'read')
      const second_read = dispatch('cc-fb', 'read')
      seedScreenshotFile(nav)
      seedScreenshotFile(first_read)
      const detail = getTask('cc-fb')!
      expect(detail.screenshotIds).toEqual([nav!, first_read!])
      expect(detail.screenshotIds).not.toContain(second_read)
    })
  })
})

describe('getTaskSummaries', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('returns one summary per requested session with the latest visual id', async () => {
    await withTempBrowserClawDir(async () => {
      for (let i = 0; i < 20; i++) {
        dispatch(`historical-${i.toString()}`, 'snapshot')
      }
      dispatch('connected-a', 'navigate', {
        url: 'https://news.example.com/story',
      })
      dispatch('connected-b', 'read', { url: 'https://docs.example.com' })
      const screenshot = dispatch('connected-a', 'screenshot')
      if (screenshot === null) throw new Error('dispatch insert failed')
      seedScreenshotFile(screenshot)

      const summaries = getTaskSummaries(['connected-a', 'missing'])

      expect([...summaries.keys()]).toEqual(['connected-a'])
      expect(summaries.get('connected-a')).toMatchObject({
        sessionId: 'connected-a',
        site: 'news.example.com',
        dispatchCount: 2,
        toolSequence: ['navigate', 'screenshot'],
        latestScreenshotId: screenshot,
      })
      expect(getTask('connected-a')?.screenshotIds).toEqual([screenshot])
    })
  })
})

describe('listTasks / getTask consistency on double end rows', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('agrees on status when a session has both errored + closed ends', () => {
    dispatch('cc-doubled', 'tabs', { url: 'https://e.com' })
    // Earliest insert wins for status; matches the real-world order
    // (transport.onerror first, then onsessionclosed).
    recordSessionEnd({
      sessionId: 'cc-doubled',
      kind: 'errored',
      reason: 'transport',
    })
    recordSessionEnd({ sessionId: 'cc-doubled', kind: 'closed' })

    const list = listTasks({}).tasks[0]!
    const detail = getTask('cc-doubled')!

    expect(list.sessionId).toBe('cc-doubled')
    expect(list.status).toBe(detail.status)
    expect(list.endedAt).toBe(detail.endedAt)
  })

  it('latestScreenshotId includes screenshots captured after errors', async () => {
    await withTempBrowserClawDir(async () => {
      dispatch('cc-last', 'tabs', { url: 'https://e.com' })
      const ok = dispatch('cc-last', 'screenshot')
      const failed = dispatch('cc-last', 'screenshot', { isError: true })
      seedScreenshotFile(ok)
      seedScreenshotFile(failed)
      const list = listTasks({}).tasks[0]!
      expect(list.latestScreenshotId).toBe(failed)
    })
  })
})
