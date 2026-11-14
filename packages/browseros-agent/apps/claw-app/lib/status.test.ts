import { describe, expect, it } from 'bun:test'
import type { LiveSessionState } from '@browseros/claw-api'
import { sessionStatusBadge } from './status'

const active: LiveSessionState = { state: 'active', browserTabs: [] }
const idle: LiveSessionState = { state: 'idle', browserTabs: [] }
const waiting: LiveSessionState = {
  state: 'idle',
  browserTabs: [],
  helpRequest: {
    requestId: 'approval-1',
    reason: 'Please approve this request before I submit it.',
    kind: 'approval',
    browserTabId: 42,
    requestedAt: 1_000,
  },
}

describe('sessionStatusBadge', () => {
  it('says Working while a live task is making tool calls', () => {
    expect(sessionStatusBadge('live', active)).toEqual({
      label: 'Working',
      badge: 'loading',
    })
    expect(sessionStatusBadge('live', undefined)).toEqual({
      label: 'Working',
      badge: 'loading',
    })
  })

  it('says Idle once a live task has gone quiet', () => {
    expect(sessionStatusBadge('live', idle)).toEqual({
      label: 'Idle',
      badge: 'neutral',
    })
  })

  it.each(['active', 'idle'] as const)(
    'says Waiting for you while help is pending despite %s activity',
    (state) => {
      expect(sessionStatusBadge('live', { ...waiting, state })).toEqual({
        label: 'Waiting for you',
        badge: 'warning',
      })
    },
  )

  it('returns to Idle after help is resolved', () => {
    expect(sessionStatusBadge('live', waiting).label).toBe('Waiting for you')
    expect(sessionStatusBadge('live', idle).label).toBe('Idle')
  })

  it('lets an ended task say how it ended whatever its last activity', () => {
    expect(sessionStatusBadge('done', idle).label).toBe('Done')
    expect(sessionStatusBadge('cancelled', idle).label).toBe('Stopped')
    expect(sessionStatusBadge('failed', waiting).label).toBe('Didn’t finish')
    expect(sessionStatusBadge('done', waiting).label).toBe('Done')
  })
})
