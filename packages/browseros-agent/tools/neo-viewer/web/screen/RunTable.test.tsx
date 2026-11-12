import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import type { RunSummary } from '../../shared/api'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { runSummary } from '../test-support/runs'
import { runTotals } from './measures'
import { RunTable } from './RunTable'
import { DEFAULT_SORT, type RunSort } from './sort'

const runs: RunSummary[] = [
  runSummary(1, {
    roundTripMs: 431,
    proxyMs: 18,
    code: "await page.goto('http://127.0.0.1:8800/form.html');\nreturn 1",
    reply: `### Result\n"${'y'.repeat(16)}"`,
  }),
  runSummary(2, {
    roundTripMs: 47,
    proxyMs: 13,
    gapMs: 6_400,
    code: 'x'.repeat(120),
    reply: `### Result\n"${'y'.repeat(888)}"`,
  }),
  runSummary(3, {
    roundTripMs: 866,
    proxyMs: 17,
    gapMs: 9_900,
    code: "await page.getByRole('button').click();",
    reply: '### Error\nwebsite / strict_mode_violation: 2 elements',
    status: 'error',
  }),
  runSummary(4, { roundTripMs: null, gapMs: 8_500, code: 'x'.repeat(164) }),
]

function Table() {
  const [sort, setSort] = useState<RunSort>(DEFAULT_SORT)
  return (
    <RunTable
      runs={runs}
      totals={runTotals(runs)}
      toolsList={{ chars: 6562, tokens: 1640 }}
      sort={sort}
      onSort={setSort}
      openIds={new Set()}
      selectedId={null}
      selectedOperation={null}
      onRow={() => {}}
      onSelectOperation={() => {}}
      onOpenFull={() => {}}
      nowMs={Date.parse(runs[3]?.sentAt ?? '') + 1_300}
    />
  )
}

describe('the run table', () => {
  let dom: TestDom
  beforeEach(async () => {
    dom = await installDom()
    await dom.render(<Table />)
  })
  afterEach(() => dom.restore())

  const order = () =>
    [...dom.container.querySelectorAll('[data-run]')].map((row) =>
      Number(row.getAttribute('data-run')),
    )
  const row = (seq: number) =>
    dom.container.querySelector(`[data-run="${seq}"]`) as Element
  const header = (label: string) =>
    dom.container.querySelector(
      `button[aria-label="Sort by ${label}"]`,
    ) as Element

  it("draws each run's Neo time as a share of the slowest, with a 2 px floor", () => {
    const width = (seq: number, bar: string) =>
      row(seq).querySelector(`[data-bar="${bar}"]`)?.getAttribute('style')
    // Neo time: 413, 34 and 849 ms; 849 fills the 110 px track.
    expect([1, 2, 3].map((seq) => width(seq, 'neo'))).toEqual([
      `width:max(2px, ${(413 / 849) * 100}%)`,
      `width:max(2px, ${(34 / 849) * 100}%)`,
      'width:max(2px, 100%)',
    ])
    // Tokens stack code then reply on one scale: run 2's ≈30 + ≈225 is the widest.
    expect(width(2, 'code')).toBe(`width:${(30 / 255) * 100}%`)
    expect(width(2, 'reply')).toBe(`width:${(225 / 255) * 100}%`)
    expect(width(1, 'reply')).toBe(`width:${(7 / 255) * 100}%`)
    expect(row(1).textContent).toContain('413 ms')
    expect(row(1).textContent).toContain('431 ms')
    // The running run shows how long it has been running, and no reply yet.
    expect(row(4).querySelector('[data-bar="neo"]')).toBeNull()
    expect(row(4).textContent).toContain('1.30 s…')
    expect(row(4).textContent).toContain('≈41…')
  })

  it('names a failed run by its error code after the clipped code', () => {
    expect(row(3).textContent).toContain('strict_mode_violation')
    expect(row(1).textContent).toContain('2 lines')
  })

  it('totals the columns, tokens marked as estimates', () => {
    const totals = dom.container.textContent ?? ''
    expect(totals).toContain('4 runs')
    expect(totals).toContain('plus ≈1,640 tok of tool description')
    // 413 + 34 + 849 ms in Neo, then 431 + 47 + 866 ms of round trips.
    expect(totals).toContain('–1.30 s1.34 s')
    // Code ≈15 + 30 + 9 + 41, replies ≈7 + 225 + 13, and both together.
    expect(totals).toContain('≈95≈245≈340 tok')
  })

  it('sorts by a header, flips on a second click, keeps a running run last', async () => {
    expect(order()).toEqual([1, 2, 3, 4])
    await dom.interact(() => fire(header('Round trip'), 'click'))
    expect(order()).toEqual([3, 1, 2, 4])
    expect(header('Round trip').textContent).toBe('Round trip ▾')
    await dom.interact(() => fire(header('Round trip'), 'click'))
    expect(order()).toEqual([2, 1, 3, 4])
    await dom.interact(() => fire(header('Reply tok'), 'click'))
    expect(order()).toEqual([2, 3, 1, 4])
    await dom.interact(() => fire(header('#'), 'click'))
    expect(order()).toEqual([1, 2, 3, 4])
  })
})
