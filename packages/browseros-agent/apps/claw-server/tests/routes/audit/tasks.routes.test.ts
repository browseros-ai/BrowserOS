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
} from '../../../src/modules/db/db'
import app from '../../../src/server'
import { recordToolDispatch } from '../../../src/services/audit-log'
import { screenshotPath } from '../../../src/services/screenshots'
import { recordSessionEnd } from '../../../src/services/session-events'
import { withTempBrowserClawDir } from '../../_helpers/temp-browserclaw-dir'

function seedScreenshotFile(dispatchId: number | null | undefined): void {
  if (typeof dispatchId !== 'number') return
  const path = screenshotPath(dispatchId)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, Buffer.from([0xff, 0xd8, 0xff, 0xd9]))
}

function seed(sessionId: string, toolName: string, url: string | null = null) {
  return recordToolDispatch({
    agentId: 'claude-code',
    slug: 'claude-code',
    agentLabel: 'Claude Code',
    sessionId,
    toolName,
    pageId: 1,
    targetId: null,
    url,
    title: null,
    rawArgs: {},
    durationMs: 5,
    result: { isError: false, structuredContent: {}, content: null },
  })
}

describe('GET /audit/tasks', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('returns 200 with the task list', async () => {
    seed('s1', 'tabs', 'https://e.com')
    seed('s1', 'snapshot')
    seed('s2', 'tabs', 'https://x.com')
    recordSessionEnd({ sessionId: 's1', kind: 'closed' })

    const res = await app.fetch(new Request('http://localhost/audit/tasks'))
    expect(res.status).toBe(200)
    const body = (await res.json()) as { tasks: unknown[] }
    expect(body.tasks).toHaveLength(2)
  })

  it('rejects bad query params with 400', async () => {
    const res = await app.fetch(
      new Request('http://localhost/audit/tasks?status=banana'),
    )
    expect(res.status).toBe(400)
  })

  it('respects the status filter', async () => {
    seed('done-1', 'tabs', 'https://e.com')
    recordSessionEnd({ sessionId: 'done-1', kind: 'closed' })
    seed('live-1', 'tabs', 'https://e.com')

    const res = await app.fetch(
      new Request('http://localhost/audit/tasks?status=done'),
    )
    const body = (await res.json()) as { tasks: { sessionId: string }[] }
    expect(body.tasks).toHaveLength(1)
    expect(body.tasks[0]!.sessionId).toBe('done-1')
  })
})

describe('GET /audit/tasks/:sessionId', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('returns 404 for unknown session', async () => {
    const res = await app.fetch(
      new Request('http://localhost/audit/tasks/nope'),
    )
    expect(res.status).toBe(404)
  })

  it('returns the full task detail', async () => {
    await withTempBrowserClawDir(async () => {
      seed('detail-1', 'tabs', 'https://e.com')
      const screenshotId = seed('detail-1', 'screenshot')
      // Simulate persistScreenshot's fire-and-forget disk write; the
      // deriver checks existence at read time.
      seedScreenshotFile(screenshotId)

      const res = await app.fetch(
        new Request('http://localhost/audit/tasks/detail-1'),
      )
      expect(res.status).toBe(200)
      const body = (await res.json()) as {
        sessionId: string
        dispatches: unknown[]
        screenshotDispatchIds: number[]
      }
      expect(body.sessionId).toBe('detail-1')
      expect(body.dispatches).toHaveLength(2)
      expect(body.screenshotDispatchIds).toHaveLength(1)
    })
  })
})
