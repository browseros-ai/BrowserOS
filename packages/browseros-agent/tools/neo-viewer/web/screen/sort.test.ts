import { describe, expect, it } from 'bun:test'
import { runSummary } from '../test-support/runs'
import { DEFAULT_SORT, nextSort, sortRuns } from './sort'

const seqs = (runs: { seq: number }[]) => runs.map((run) => run.seq)

describe('sorting the runs', () => {
  const runs = [
    runSummary(1, {
      roundTripMs: 431,
      reply: `### Result\n"${'y'.repeat(16)}"`,
    }),
    runSummary(2, {
      roundTripMs: 47,
      reply: `### Result\n"${'y'.repeat(888)}"`,
    }),
    runSummary(3, {
      roundTripMs: 133,
      reply: `### Result\n"${'y'.repeat(48)}"`,
    }),
    runSummary(4, { roundTripMs: null }),
    runSummary(5, {
      roundTripMs: 866,
      reply: `### Result\n"${'y'.repeat(32)}"`,
    }),
  ]

  it('starts in the order the calls arrived', () => {
    expect(seqs(sortRuns([...runs].reverse(), DEFAULT_SORT))).toEqual([
      1, 2, 3, 4, 5,
    ])
  })

  it('sorts by a column either way, a run still running always last', () => {
    expect(
      seqs(sortRuns(runs, { key: 'roundTrip', direction: 'desc' })),
    ).toEqual([5, 1, 3, 2, 4])
    expect(
      seqs(sortRuns(runs, { key: 'roundTrip', direction: 'asc' })),
    ).toEqual([2, 3, 1, 5, 4])
    expect(seqs(sortRuns(runs, { key: 'reply', direction: 'desc' }))).toEqual([
      2, 3, 5, 1, 4,
    ])
  })

  it('keeps call order between equal values', () => {
    const same = [
      runSummary(3, { code: 'aaaa' }),
      runSummary(1, { code: 'bbbb' }),
      runSummary(2, { code: 'cccc' }),
    ]
    expect(seqs(sortRuns(same, { key: 'code', direction: 'desc' }))).toEqual([
      1, 2, 3,
    ])
  })

  it('flips the direction on the same header and starts a new column largest first', () => {
    expect(nextSort(DEFAULT_SORT, 'seq')).toEqual({
      key: 'seq',
      direction: 'desc',
    })
    expect(nextSort(DEFAULT_SORT, 'neo')).toEqual({
      key: 'neo',
      direction: 'desc',
    })
    expect(nextSort({ key: 'neo', direction: 'desc' }, 'neo')).toEqual({
      key: 'neo',
      direction: 'asc',
    })
    expect(nextSort({ key: 'neo', direction: 'asc' }, 'seq')).toEqual(
      DEFAULT_SORT,
    )
  })
})
