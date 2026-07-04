/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration test for the POST /agents/:agentId/cancel route. The
 * route reads from the process-wide dispatchCancellation singleton
 * which sources its identityService from src/lib/mcp-session. We
 * drive it by registering identities + AbortControllers directly,
 * then issuing the HTTP call and asserting the wire shape + abort
 * propagation.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import {
  agentIdentityFromClient,
  identityService,
} from '../../../src/lib/mcp-session'
import app, { type AppType } from '../../../src/server'
import { dispatchCancellation } from '../../../src/services/dispatch-cancellation'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

afterEach(() => {
  dispatchCancellation.clear()
  identityService.clear()
})

describe('POST /agents/:agentId/cancel', () => {
  test('returns 404 with cancelled: 0 when no sessions match', async () => {
    const res = await client().agents[':agentId'].cancel.$post({
      param: { agentId: 'nobody-home' },
    })
    expect(res.status).toBe(404)
    const body = (await res.json()) as { ok: boolean; cancelled: number }
    expect(body.ok).toBe(false)
    expect(body.cancelled).toBe(0)
  })

  test('aborts every registered controller for the agent and returns the count', async () => {
    const identity = identityService.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'claude-code', version: '0.1.0' },
    })
    const c1 = new AbortController()
    const c2 = new AbortController()
    dispatchCancellation.register(identity.sessionId, c1)
    dispatchCancellation.register(identity.sessionId, c2)

    const res = await client().agents[':agentId'].cancel.$post({
      param: { agentId: agentIdentityFromClient(identity).agentId },
    })

    expect(res.status).toBe(200)
    const body = (await res.json()) as { ok: boolean; cancelled: number }
    expect(body.ok).toBe(true)
    expect(body.cancelled).toBe(2)
    expect(c1.signal.aborted).toBe(true)
    expect(c2.signal.aborted).toBe(true)
    expect(c1.signal.reason).toBe('Operation cancelled by the User')
  })

  test('does not abort controllers belonging to a different agent', async () => {
    const a = identityService.registerInitialize({
      sessionId: 'sA',
      clientInfo: { name: 'claude-code', version: '0.1.0' },
    })
    const b = identityService.registerInitialize({
      sessionId: 'sB',
      clientInfo: { name: 'cursor', version: '0.1.0' },
    })
    const cA = new AbortController()
    const cB = new AbortController()
    dispatchCancellation.register(a.sessionId, cA)
    dispatchCancellation.register(b.sessionId, cB)

    const res = await client().agents[':agentId'].cancel.$post({
      param: { agentId: agentIdentityFromClient(a).agentId },
    })

    expect(res.status).toBe(200)
    const body = (await res.json()) as { ok: boolean; cancelled: number }
    expect(body.cancelled).toBe(1)
    expect(cA.signal.aborted).toBe(true)
    expect(cB.signal.aborted).toBe(false)
  })
})
