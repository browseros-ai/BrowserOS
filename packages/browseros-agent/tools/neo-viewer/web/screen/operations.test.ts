import { describe, expect, it } from 'bun:test'
import { operation, record } from '../test-support/operations'
import {
  namesWithCounts,
  placeOperations,
  spanOf,
  waterfallBar,
} from './operations'

describe('placing operations on the script lines', () => {
  // As Neo recorded the artboard's run 3 and a loop: background work first, then the script's.
  const run = record([
    operation(1, 'events.next', { startOffsetMs: 23, durationMs: 56 }),
    operation(2, 'page.frames', { startOffsetMs: 23 }),
    operation(5, 'locator.fill', { line: 1, startOffsetMs: 24, durationMs: 9 }),
    operation(7, 'locator.textContent', { line: 3, startOffsetMs: 33 }),
    operation(6, 'locator.count', { line: 3, startOffsetMs: 31 }),
    operation(8, 'locator.textContent', { line: 3, startOffsetMs: 33 }),
    operation(9, 'locator.click', {
      parentId: 'neo-run.5',
      startOffsetMs: 25,
    }),
    operation(10, 'page.title', { line: 9, startOffsetMs: 40 }),
  ])

  it('puts each operation on the line that called it, in start order', () => {
    const placed = placeOperations(run, 4)
    expect([...placed.byLine.keys()].sort()).toEqual([1, 3])
    expect(placed.byLine.get(1)?.map((op) => op.seq)).toEqual([5])
    // A loop's calls stay on their one line, ordered by start, then by seq.
    expect(placed.byLine.get(3)?.map((op) => op.seq)).toEqual([6, 7, 8])
  })

  it('groups operations with no line apart: no site, a nested call, a line past the script', () => {
    const placed = placeOperations(run, 4)
    expect(placed.unplaced.map((op) => op.seq)).toEqual([1, 2, 9, 10])
    expect(namesWithCounts(placed.unplaced)).toBe(
      'events.next, page.frames, locator.click, page.title',
    )
    expect(
      namesWithCounts([
        operation(1, 'events.listen'),
        operation(2, 'page.frames'),
        operation(3, 'events.listen'),
      ]),
    ).toBe('events.listen ×2, page.frames')
  })

  it("draws bars inside the run's own time", () => {
    const placed = placeOperations(run, 4)
    expect(placed.spanMs).toBe(100)
    // The artboard's run 3: 118 ms long, selectOption from +24 ms for 31 ms.
    expect(
      waterfallBar(
        operation(6, 'locator.selectOption', {
          startOffsetMs: 24,
          durationMs: 31,
        }),
        118,
      ),
    ).toEqual({ left: 24 / 118, width: 31 / 118, open: false })
  })

  it('keeps an unfinished operation, and an open record, inside the track', () => {
    const open = operation(3, 'page.waitForTimeout', {
      startOffsetMs: 60,
      durationMs: null,
    })
    expect(waterfallBar(open, 100)).toEqual({
      left: 0.6,
      width: 0.4,
      open: true,
    })
    // A record still running has no duration: the span is as far as an operation reached.
    expect(
      spanOf(
        record(
          [
            operation(1, 'a', { startOffsetMs: 10, durationMs: 30 }),
            operation(2, 'b', { startOffsetMs: 50, durationMs: null }),
          ],
          null,
        ),
      ),
    ).toBe(50)
    // An operation reported past the run's end is clipped at the end of the track.
    const late = waterfallBar(
      operation(4, 'c', { startOffsetMs: 90, durationMs: 40 }),
      100,
    )
    expect(late.left).toBe(0.9)
    expect(late.width).toBeCloseTo(0.1, 10)
    expect(spanOf(record([], 0))).toBe(1)
  })
})
