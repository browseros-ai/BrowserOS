import { describe, expect, it } from 'bun:test'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { mapDispatchToFrame, mapTaskStatus } from './replay.data'

function dispatch(createdAt: number, durationMs?: number): ToolDispatchRow {
  return {
    dispatchId: 42,
    createdAt,
    slug: 'codex',
    label: 'Codex',
    sessionId: 'session-1',
    toolName: 'read',
    tabId: 7,
    durationMs,
  }
}

describe('mapTaskStatus', () => {
  it('maps cancelled API sessions to the existing stopped run status', () => {
    expect(mapTaskStatus('cancelled')).toBe('stopped')
  })
})

describe('mapDispatchToFrame', () => {
  it('keeps completion time for the timeline and derives camera time from duration', () => {
    const frame = mapDispatchToFrame(dispatch(15_000, 4_000), 1_000, new Map())

    expect(frame.t).toBe(14)
    expect(frame.cameraT).toBe(10)
  })

  it.each([undefined, -1, Number.NaN, Number.POSITIVE_INFINITY])(
    'falls back to completion for unusable duration %p',
    (durationMs) => {
      const frame = mapDispatchToFrame(
        dispatch(15_000, durationMs),
        1_000,
        new Map(),
      )

      expect(frame.t).toBe(14)
      expect(frame.cameraT).toBe(14)
    },
  )

  it('clamps an operation start at the beginning of the session', () => {
    const frame = mapDispatchToFrame(dispatch(2_000, 5_000), 1_000, new Map())

    expect(frame.t).toBe(1)
    expect(frame.cameraT).toBe(0)
  })
})
