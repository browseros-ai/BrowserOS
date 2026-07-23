import { describe, expect, it } from 'bun:test'
import { mapTaskStatus } from './replay.data'

describe('mapTaskStatus', () => {
  it('maps cancelled API sessions to the existing stopped run status', () => {
    expect(mapTaskStatus('cancelled')).toBe('stopped')
  })
})
