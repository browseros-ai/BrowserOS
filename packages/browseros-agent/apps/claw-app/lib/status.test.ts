import { describe, expect, it } from 'bun:test'
import { sessionStatusBadge } from './status'

describe('sessionStatusBadge', () => {
  it('says Working while a live task is making tool calls', () => {
    expect(sessionStatusBadge('live', 'active')).toEqual({
      label: 'Working',
      badge: 'loading',
    })
    expect(sessionStatusBadge('live', undefined)).toEqual({
      label: 'Working',
      badge: 'loading',
    })
  })

  it('says Idle once a live task has gone quiet', () => {
    expect(sessionStatusBadge('live', 'idle')).toEqual({
      label: 'Idle',
      badge: 'neutral',
    })
  })

  it('lets an ended task say how it ended whatever its last activity', () => {
    expect(sessionStatusBadge('done', 'idle').label).toBe('Done')
    expect(sessionStatusBadge('cancelled', 'idle').label).toBe('Stopped')
  })
})
