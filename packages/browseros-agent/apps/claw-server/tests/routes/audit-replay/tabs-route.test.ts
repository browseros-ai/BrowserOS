/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration test for `GET /replay/tabs`. Drives the real
 * singletons (registry, identity service, tab-group tracker) by
 * registering identities + tool dispatches, then hits the route via
 * the Hono client.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import {
  agentIdentityFromClient,
  identityService,
} from '../../../src/lib/mcp-session'
import { tabActivityRegistry } from '../../../src/lib/tab-activity'
import app, { type AppType } from '../../../src/server'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

afterEach(() => {
  tabActivityRegistry.clear()
  identityService.clear()
})

describe('GET /replay/tabs', () => {
  test('returns an empty list when no agents are active', async () => {
    const res = await client().replay.tabs.$get()
    expect(res.status).toBe(200)
    const body = (await res.json()) as { tabs: unknown[] }
    expect(body).toEqual({ tabs: [] })
  })

  test('returns the agent tab row when a session + dispatch are live', async () => {
    const identity = identityService.registerInitialize({
      sessionId: 'sid-abc',
      clientInfo: { name: 'claude-code', version: '0.1.0' },
    })
    const { agentId, slug } = agentIdentityFromClient(identity)
    // Drive a tool dispatch to populate the registry. The registry
    // needs a session attached to evaluate `status`, so this test
    // mounts a stub session via setBrowserSession.
    const { setBrowserSession } = await import(
      '../../../src/lib/browser-session'
    )
    setBrowserSession({
      pages: {
        getInfo: (pageId: number) =>
          pageId === 5
            ? {
                targetId: 'cdp-target-5',
                url: 'https://news.google.com/',
                title: 'Top stories',
              }
            : undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    tabActivityRegistry.recordTool({
      agentId,
      slug,
      pageId: 5,
      targetId: 'cdp-target-5',
      toolName: 'tabs',
    })

    const res = await client().replay.tabs.$get()
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      tabs: Array<{
        sessionId: string
        tabPageId: number
        url: string
        title: string
        groupColor: string | null
      }>
    }
    expect(body.tabs).toHaveLength(1)
    expect(body.tabs[0]).toMatchObject({
      sessionId: 'sid-abc',
      tabPageId: 5,
      url: 'https://news.google.com/',
      title: 'Top stories',
    })
    // groupColor is null until ensureAgentTabGroup creates the group;
    // we did not call that here, so null is expected.
    expect(body.tabs[0].groupColor).toBeNull()

    setBrowserSession(null)
  })

  test('drops a tab whose identity is no longer live', async () => {
    const { setBrowserSession } = await import(
      '../../../src/lib/browser-session'
    )
    setBrowserSession({
      pages: {
        getInfo: () => ({
          targetId: 'cdp-target-9',
          url: 'https://x.com/',
          title: 'X',
        }),
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    tabActivityRegistry.recordTool({
      agentId: 'ghost',
      slug: 'ghost',
      pageId: 9,
      targetId: 'cdp-target-9',
      toolName: 'tabs',
    })
    // No identity registered for 'ghost'.
    const res = await client().replay.tabs.$get()
    const body = (await res.json()) as { tabs: unknown[] }
    expect(body.tabs).toEqual([])
    setBrowserSession(null)
  })
})
