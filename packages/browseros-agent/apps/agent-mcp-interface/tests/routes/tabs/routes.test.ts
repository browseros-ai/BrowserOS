/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration test for the /tabs/activity route. Pins the response
 * shape and the empty-state behaviour. The registry-population path
 * is exercised by mcp/register tests; this file only verifies the
 * route surface.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import { setBrowserSession } from '../../../src/lib/browser-session'
import { tabActivityRegistry } from '../../../src/lib/tab-activity'
import app, { type AppType } from '../../../src/server'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

afterEach(() => {
  // Clear the singleton registry between cases so test ordering does
  // not leak state. Setting the session to null short-circuits
  // `snapshot()` but does NOT empty the underlying records Map; only
  // the explicit `clear()` does that. Skipping it would leave a stale
  // record visible to a later test that re-attaches a session whose
  // stub resolves the same pageId.
  tabActivityRegistry.clear()
  setBrowserSession(null)
})

describe('/tabs/activity route', () => {
  test('returns an empty list when nothing has been recorded', async () => {
    const api = client()
    const res = await api.tabs.activity.$get()
    expect(res.status).toBe(200)
    const body = (await res.json()) as { tabs: unknown[] }
    expect(body).toEqual({ tabs: [] })
  })

  test('returns the registry snapshot once tools have been recorded', async () => {
    // Plant a fake session whose PageManager resolves a single page,
    // record a tool against it, and expect the route to surface it.
    setBrowserSession({
      pages: {
        getInfo: (pageId: number) =>
          pageId === 1
            ? { targetId: 't1', url: 'https://example.com/', title: 'Ex' }
            : undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: stub for test
    } as any)
    tabActivityRegistry.recordTool({
      agentId: 'a-1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    const api = client()
    const res = await api.tabs.activity.$get()
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      tabs: Array<{
        targetId: string
        agentId: string
        slug: string
        toolName?: string
        lastToolName: string
        url: string
        status: 'active' | 'idle'
      }>
    }
    expect(body.tabs).toHaveLength(1)
    expect(body.tabs[0]).toMatchObject({
      targetId: 't1',
      agentId: 'a-1',
      slug: 'finance-ops',
      lastToolName: 'navigate',
      url: 'https://example.com/',
      status: 'active',
    })
  })
})
