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
import {
  agentIdentityFromClient,
  type ClientIdentity,
  type IdentityService,
} from '../../src/lib/mcp-session'
import { createDispatchCancellation } from '../../src/services/dispatch-cancellation'

function makeIdentity(over: Partial<ClientIdentity>): ClientIdentity {
  return {
    sessionId: 's1',
    clientName: 'claude-code',
    clientVersion: '0.1.0',
    clientTitle: null,
    sessionLabel: null,
    firstSeenAt: 1_000_000,
    ...over,
  }
}

function stubIdentityService(
  identities: ClientIdentity[],
): Pick<IdentityService, 'list'> {
  return { list: () => identities }
}

function agentIdFor(identity: ClientIdentity): string {
  return agentIdentityFromClient(identity).agentId
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
    const matching = makeIdentity({
      sessionId: 's1',
      clientName: 'claude-code',
    })
    const sameNameOtherSession = makeIdentity({
      sessionId: 's2',
      clientName: 'claude-code',
    })
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([
        matching,
        sameNameOtherSession,
        makeIdentity({ sessionId: 's3', clientName: 'cursor' }),
      ]),
    })
    const c1 = new AbortController()
    const c2 = new AbortController()
    const cSameName = new AbortController()
    const cOther = new AbortController()
    svc.register('s1', c1)
    svc.register('s1', c2)
    svc.register('s2', cSameName)
    svc.register('s3', cOther)

    const cancelled = svc.cancelByAgent(agentIdFor(matching), 'stop now')

    expect(cancelled).toBe(2)
    expect(c1.signal.aborted).toBe(true)
    expect(c2.signal.aborted).toBe(true)
    expect(cSameName.signal.aborted).toBe(false)
    expect(cOther.signal.aborted).toBe(false)
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
    const identity = makeIdentity({
      sessionId: 's1',
      clientName: 'claude-code',
    })
    const svc = createDispatchCancellation({
      identityService: stubIdentityService([identity]),
    })
    expect(svc.cancelByAgent(agentIdFor(identity), 'no')).toBe(0)
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
