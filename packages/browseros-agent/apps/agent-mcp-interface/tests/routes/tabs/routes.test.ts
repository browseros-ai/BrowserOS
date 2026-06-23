/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration test for the /tabs/activity route. Pins the response
 * shape, the empty-state behaviour, and the agent-profile join.
 * The registry-population path is exercised by mcp/register tests;
 * this file only verifies the route surface.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import { setBrowserSession } from '../../../src/lib/browser-session'
import { tabActivityRegistry } from '../../../src/lib/tab-activity'
import { create as createAgent } from '../../../src/routes/agents/service'
import app, { type AppType } from '../../../src/server'
import { withTempBrowserosDir } from '../../_helpers/temp-browseros-dir'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

function stubSession() {
  setBrowserSession({
    pages: {
      getInfo: (pageId: number) =>
        pageId === 1
          ? { targetId: 't1', url: 'https://example.com/', title: 'Ex' }
          : undefined,
    },
    // biome-ignore lint/suspicious/noExplicitAny: stub for test
  } as any)
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

  test('returns the enriched record once a tool has been recorded', async () => {
    await withTempBrowserosDir(async () => {
      // Seed a real agent profile on disk so the route's join finds
      // a label + harness instead of falling back.
      const agent = await createAgent({
        name: 'Finance Ops',
        harness: 'Claude Code',
        loginMode: 'profile',
        selectedSites: ['stripe.com'],
        approvals: {
          submit: 'Ask',
          payment: 'Block',
          delete: 'Ask',
          upload: 'Ask',
          navigate: 'Auto',
          input: 'Auto',
        },
        aclRuleIds: [],
        customAclRules: [],
      })
      stubSession()
      tabActivityRegistry.recordTool({
        agentId: agent.id,
        slug: agent.slug,
        pageId: 1,
        targetId: 't1',
        toolName: 'navigate',
      })
      const res = await client().tabs.activity.$get()
      expect(res.status).toBe(200)
      const body = (await res.json()) as {
        tabs: Array<{
          targetId: string
          agentId: string
          slug: string
          firstToolAt: number
          lastToolAt: number
          lastToolName: string
          toolCount: number
          recentTools: Array<{ name: string; at: number }>
          agentLabel: string
          harness: string | null
          color: string | null
          status: 'active' | 'idle'
        }>
      }
      expect(body.tabs).toHaveLength(1)
      const row = body.tabs[0]
      expect(row).toMatchObject({
        targetId: 't1',
        agentId: agent.id,
        slug: agent.slug,
        lastToolName: 'navigate',
        toolCount: 1,
        agentLabel: 'Finance Ops',
        harness: 'Claude Code',
        color: null,
        status: 'active',
      })
      expect(row.firstToolAt).toBe(row.lastToolAt)
      expect(row.recentTools).toEqual([
        { name: 'navigate', at: row.lastToolAt },
      ])
    })
  })

  test('falls back to slug when the agent profile is missing', async () => {
    await withTempBrowserosDir(async () => {
      // No profile on disk: the route should not throw, and should
      // surface the slug as a fallback label with null harness/color.
      stubSession()
      tabActivityRegistry.recordTool({
        agentId: 'unknown',
        slug: 'orphan-slug',
        pageId: 1,
        targetId: 't1',
        toolName: 'navigate',
      })
      const res = await client().tabs.activity.$get()
      expect(res.status).toBe(200)
      const body = (await res.json()) as {
        tabs: Array<{
          agentLabel: string
          harness: string | null
          color: string | null
        }>
      }
      expect(body.tabs).toHaveLength(1)
      expect(body.tabs[0].agentLabel).toBe('orphan-slug')
      expect(body.tabs[0].harness).toBeNull()
      expect(body.tabs[0].color).toBeNull()
    })
  })
})
