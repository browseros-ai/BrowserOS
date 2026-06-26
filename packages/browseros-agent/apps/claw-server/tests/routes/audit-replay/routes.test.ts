/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration tests for /audit/replay/:sessionId/{events, exists, ""}.
 * Each test runs inside withTempBrowserosDir so the replay-storage
 * singleton lays its files down under an isolated tmp directory.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import { identityService } from '../../../src/lib/mcp-session'
import app, { type AppType } from '../../../src/server'
import { replayStorage } from '../../../src/services/replay-storage'
import { withTempBrowserosDir } from '../../_helpers/temp-browseros-dir'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

function registerLiveSession(sessionId: string, clientName = 'tester'): void {
  identityService.registerInitialize({
    sessionId,
    clientInfo: { name: clientName, version: '0.0.1' },
  })
}

function ndjsonLine(over: {
  tabPageId?: number
  ts?: number
  type?: number
  data?: unknown
}): string {
  return JSON.stringify({
    tabPageId: over.tabPageId ?? 1,
    ts: over.ts ?? Date.now(),
    type: over.type ?? 3,
    data: over.data ?? { foo: 'bar' },
  })
}

afterEach(async () => {
  await replayStorage.resetForTesting()
  identityService.clear()
})

describe('audit-replay routes', () => {
  test('POST events from a live session writes lines, GET reads them back', async () => {
    await withTempBrowserosDir(async () => {
      registerLiveSession('s1')
      const body = [
        ndjsonLine({ tabPageId: 1, ts: 100, type: 2 }),
        ndjsonLine({ tabPageId: 1, ts: 200, type: 3 }),
      ].join('\n')
      const post = await client().audit.replay[':sessionId'].events.$post(
        { param: { sessionId: 's1' } },
        {
          init: {
            method: 'POST',
            headers: { 'content-type': 'application/x-ndjson' },
            body,
          },
        },
      )
      expect(post.status).toBe(200)
      const accept = (await post.json()) as { ok: boolean; accepted: number }
      expect(accept).toEqual({ ok: true, accepted: 2 })

      const stream = await client().audit.replay[':sessionId'].$get({
        param: { sessionId: 's1' },
      })
      expect(stream.status).toBe(200)
      expect(stream.headers.get('content-type')).toContain(
        'application/x-ndjson',
      )
      const text = await stream.text()
      const lines = text.split('\n').filter(Boolean)
      expect(lines).toHaveLength(2)
      expect(JSON.parse(lines[0]).sessionId).toBe('s1')
      expect(JSON.parse(lines[0]).ts).toBe(100)
    })
  })

  test('POST returns 410 for a session whose identity is not live', async () => {
    await withTempBrowserosDir(async () => {
      const post = await client().audit.replay[':sessionId'].events.$post(
        { param: { sessionId: 'ghost' } },
        { init: { method: 'POST', body: ndjsonLine({}) } },
      )
      expect(post.status).toBe(410)
      const json = (await post.json()) as { ok: boolean; reason: string }
      expect(json.ok).toBe(false)
      expect(json.reason).toBe('session not live')
    })
  })

  test('GET returns 404 for an unknown sessionId', async () => {
    await withTempBrowserosDir(async () => {
      const res = await client().audit.replay[':sessionId'].$get({
        param: { sessionId: 'no-such-session' },
      })
      expect(res.status).toBe(404)
    })
  })

  test('GET /exists returns hasData:false for unknown session', async () => {
    await withTempBrowserosDir(async () => {
      const res = await client().audit.replay[':sessionId'].exists.$get({
        param: { sessionId: 'no-such-session' },
      })
      expect(res.status).toBe(200)
      const json = (await res.json()) as {
        ok: boolean
        hasData: boolean
        sizeBytes: number
        tabPageIds: number[]
      }
      expect(json.ok).toBe(true)
      expect(json.hasData).toBe(false)
      expect(json.sizeBytes).toBe(0)
      expect(json.tabPageIds).toEqual([])
    })
  })

  test('GET /exists returns tabPageIds + first/last timestamps once populated', async () => {
    await withTempBrowserosDir(async () => {
      registerLiveSession('s2')
      await client().audit.replay[':sessionId'].events.$post(
        { param: { sessionId: 's2' } },
        {
          init: {
            method: 'POST',
            body: [
              ndjsonLine({ tabPageId: 4, ts: 1_000 }),
              ndjsonLine({ tabPageId: 4, ts: 2_000 }),
              ndjsonLine({ tabPageId: 9, ts: 3_000 }),
            ].join('\n'),
          },
        },
      )
      const res = await client().audit.replay[':sessionId'].exists.$get({
        param: { sessionId: 's2' },
      })
      const json = (await res.json()) as {
        hasData: boolean
        firstEventAt: number
        lastEventAt: number
        tabPageIds: number[]
      }
      expect(json.hasData).toBe(true)
      expect(json.firstEventAt).toBe(1_000)
      expect(json.lastEventAt).toBe(3_000)
      expect(json.tabPageIds.sort((a, b) => a - b)).toEqual([4, 9])
    })
  })

  test('POST with empty body returns 200 accepted:0', async () => {
    await withTempBrowserosDir(async () => {
      registerLiveSession('s3')
      const res = await client().audit.replay[':sessionId'].events.$post(
        { param: { sessionId: 's3' } },
        { init: { method: 'POST', body: '' } },
      )
      expect(res.status).toBe(200)
      const json = (await res.json()) as { accepted: number }
      expect(json.accepted).toBe(0)
    })
  })
})
