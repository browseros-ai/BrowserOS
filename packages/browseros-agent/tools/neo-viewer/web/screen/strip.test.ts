import { describe, expect, it } from 'bun:test'
import { runSummary } from '../test-support/runs'
import { stripLayout, ticks } from './strip'
import { clock } from './units'

describe('the "where the time went" strip', () => {
  it('places each run where it sat in the wall clock, a running one reaching now', () => {
    const runs = [
      runSummary(1, { at: 0, roundTripMs: 400, proxyMs: 100 }),
      runSummary(2, { at: 6_000, roundTripMs: 100, status: 'error' }),
      runSummary(3, { at: 9_000, roundTripMs: null, proxyMs: 200 }),
    ]
    const start = Date.parse(runs[0]?.sentAt ?? '')
    const layout = stripLayout(runs, start + 10_000, 'run-2')
    expect(layout?.live).toBe(true)
    expect(layout?.endLabel).toBe('now')
    // The viewer's hold before forwarding is kept apart from Neo's answer.
    expect(
      layout?.blocks.map(({ left, hold, width, tone }) => ({
        left,
        hold,
        width,
        tone,
      })),
    ).toEqual([
      { left: 0, hold: 0.01, width: 0.03, tone: 'neo' },
      { left: 0.6, hold: 0, width: 0.01, tone: 'bad' },
      { left: 0.9, hold: 0.02, width: 0.08, tone: 'busy' },
    ])
  })

  it('ends at the last reply when nothing is running', () => {
    const runs = [
      runSummary(1, { at: 0, roundTripMs: 400 }),
      runSummary(2, { at: 1_000, roundTripMs: 1_000 }),
    ]
    const layout = stripLayout(runs, Date.now())
    expect(layout?.live).toBe(false)
    expect((layout?.endMs ?? 0) - (layout?.startMs ?? 0)).toBe(2_000)
    expect(layout?.endLabel).toBe(clock(layout?.endMs ?? 0))
    expect(stripLayout([], Date.now())).toBeNull()
  })

  it('prints round clock marks, few enough to read, clear of the end labels', () => {
    // The design's task: 10:14:02 to 10:15:16, marked at 10:14:30 and 10:15:00.
    const start = new Date(2026, 10, 11, 10, 14, 2).getTime()
    const marks = ticks(start, start + 74_000)
    expect(marks.map((mark) => mark.label)).toEqual(['10:14:30', '10:15:00'])
    expect(marks[0]?.left).toBeCloseTo(28 / 74, 5)
    // A mark 1 s from the start would sit on the start label; it is left out.
    const near = ticks(start - 1_000, start - 1_000 + 20_000)
    expect(near.every((mark) => mark.left >= 0.06 && mark.left <= 0.92)).toBe(
      true,
    )
  })

  it('labels dense runs every few, always the selected and the last', () => {
    // 200 runs 20 ms apart: each 0.5% of the width from the next, too close to label them all.
    const runs = Array.from({ length: 200 }, (_, i) =>
      runSummary(i + 1, { at: i * 20, roundTripMs: 5 }),
    )
    const layout = stripLayout(runs, Date.now(), 'run-2')
    const labelled = layout?.blocks
      .filter((block) => block.labelled)
      .map((block) => block.seq)
    expect(labelled).toContain(1)
    expect(labelled).toContain(2)
    expect(labelled).toContain(200)
    expect(labelled?.length).toBeLessThan(100)
  })
})
