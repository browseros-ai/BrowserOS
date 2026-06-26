/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit tests for the dispatch-cancellation service. We construct
 * factory instances with a stub identity service so the tests do not
 * touch the process-wide singleton.
 */

import { describe, expect, it } from 'bun:test'
import type { ClientIdentity, IdentityService } from '../../src/lib/mcp-session'
import { createDispatchCancellation } from '../../src/services/dispatch-cancellation'

function makeIdentity(over: Partial<ClientIdentity>): ClientIdentity {
  return {
    sessionId: 's1',
    clientName: 'claude-code',
    clientVersion: '0.1.0',
    clientTitle: null,
    firstSeenAt: 1_000_000,
    ...over,
  }
}

function stubIdentityService(
  identities: ClientIdentity[],
): Pick<IdentityService, 'list'> {
  return { list: () => identities }
}

describe('dispatch-cancellation', () => {
  it('register + unregister round-trips controllers and prunes empty sets', () => {
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([]),
    })
    const ctrl1 = new AbortController()
    const ctrl2 = new AbortController()
    svc.register('s1', ctrl1)
    svc.register('s1', ctrl2)
    expect(svc.size()).toBe(2)
    svc.unregister('s1', ctrl1)
    expect(svc.size()).toBe(1)
    svc.unregister('s1', ctrl2)
    expect(svc.size()).toBe(0)
  })

  it('unregister is a no-op for unknown sessionId', () => {
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([]),
    })
    expect(() => svc.unregister('unknown', new AbortController())).not.toThrow()
  })

  it('cancelByAgent aborts every controller for matching sessions', () => {
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([
        makeIdentity({ sessionId: 's1', clientName: 'claude-code' }),
        makeIdentity({ sessionId: 's2', clientName: 'claude-code' }),
        makeIdentity({ sessionId: 's3', clientName: 'cursor' }),
      ]),
    })
    const c1 = new AbortController()
    const c2 = new AbortController()
    const c3 = new AbortController()
    svc.register('s1', c1)
    svc.register('s2', c2)
    svc.register('s3', c3)

    // agentIdentityFromClient slugifies clientName, so 'claude-code'
    // becomes agentId 'claude-code'.
    const cancelled = svc.cancelByAgent('claude-code', 'stop now')

    expect(cancelled).toBe(2)
    expect(c1.signal.aborted).toBe(true)
    expect(c2.signal.aborted).toBe(true)
    expect(c3.signal.aborted).toBe(false)
    expect(c1.signal.reason).toBe('stop now')
  })

  it('cancelByAgent returns 0 when no sessions match', () => {
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([
        makeIdentity({ sessionId: 's1', clientName: 'claude-code' }),
      ]),
    })
    const c1 = new AbortController()
    svc.register('s1', c1)
    expect(svc.cancelByAgent('ghost-agent', 'no')).toBe(0)
    expect(c1.signal.aborted).toBe(false)
  })

  it('cancelByAgent returns 0 when the matching session has no controllers', () => {
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([
        makeIdentity({ sessionId: 's1', clientName: 'claude-code' }),
      ]),
    })
    expect(svc.cancelByAgent('claude-code', 'no')).toBe(0)
  })

  it('clear empties the registry', () => {
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([]),
    })
    svc.register('s1', new AbortController())
    svc.register('s2', new AbortController())
    expect(svc.size()).toBe(2)
    svc.clear()
    expect(svc.size()).toBe(0)
  })
})
