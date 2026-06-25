/**
 * Walks the focus route against the in-process Hono app. Mocks the
 * orchestration service so the test does not need a real browser
 * session attached.
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'

// Same isolation note as tab-group-ops.test.ts: only mock the
// framework, not the registry, so the real BROWSER_TOOLS catalogue
// stays intact for other suites in the same `bun test` run.
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  executeTool: async () => ({
    isError: false,
    content: [{ type: 'text', text: 'ok' }],
  }),
}))

const { setBrowserSession } = await import('../../../src/lib/browser-session')
const { tabGroupTracker } = await import('../../../src/lib/agent-tab-groups')
const app = (await import('../../../src/server')).default

describe('POST /cockpit/tabs/focus/:agentId', () => {
  beforeEach(() => {
    tabGroupTracker.reset()
  })
  afterEach(() => {
    tabGroupTracker.reset()
    setBrowserSession(null)
  })

  it('returns 503 when no browser session is attached', async () => {
    setBrowserSession(null)
    const res = await app.fetch(
      new Request('http://localhost/tabs/focus/claude-code', {
        method: 'POST',
      }),
    )
    expect(res.status).toBe(503)
    const body = (await res.json()) as { ok: boolean; reason: string }
    expect(body.ok).toBe(false)
    expect(body.reason).toContain('browser session')
  })

  it('returns 404 when the agent has no tracked group', async () => {
    setBrowserSession({} as never)
    const res = await app.fetch(
      new Request('http://localhost/tabs/focus/unknown', { method: 'POST' }),
    )
    expect(res.status).toBe(404)
    const body = (await res.json()) as { ok: boolean }
    expect(body.ok).toBe(false)
  })

  it('returns 200 ok with groupId + windowId on the happy path', async () => {
    setBrowserSession({} as never)
    tabGroupTracker.recordOpen({
      agentId: 'claude-code',
      slug: 'claude-code',
      pageId: 1,
    })
    tabGroupTracker.rememberGroup({
      agentId: 'claude-code',
      groupId: 'G1',
      windowId: 42,
    })
    const res = await app.fetch(
      new Request('http://localhost/tabs/focus/claude-code', {
        method: 'POST',
      }),
    )
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      ok: boolean
      groupId: string
      windowId: number
    }
    expect(body).toEqual({ ok: true, groupId: 'G1', windowId: 42 })
  })
})
