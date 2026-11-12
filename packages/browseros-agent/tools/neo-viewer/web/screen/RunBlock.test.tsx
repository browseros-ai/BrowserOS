import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { RunDetail, RunSummary } from '../../shared/api'
import { formatCode } from '../../shared/code-format'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { operation, record } from '../test-support/operations'
import { runDetail, runSummary } from '../test-support/runs'
import { RunBlock } from './RunBlock'

const code = [
  "await page.getByRef('e5').fill('Grace Hopper');",
  'const labels = [];',
  "for (const b of await page.getByRole('button').all()) labels.push(await b.textContent());",
].join('\n')

const fill = operation(5, 'locator.fill', {
  line: 1,
  effect: 'write',
  startOffsetMs: 24,
  durationMs: 9,
  delivery: 'done',
  args: {
    call: { api: 'locator.fill', timeoutMs: 5000 },
    target: { $neo: 'locator', display: "locator('aria-ref=e5')" },
    value: 'Grace Hopper',
  },
  argsTruncated: true,
  result: 'x'.repeat(10),
  resultTruncated: true,
})
// A loop on line 3: the count, then one textContent per button.
const loop = [
  operation(6, 'locator.count', { line: 3, startOffsetMs: 40 }),
  ...[7, 8, 9, 10].map((seq) =>
    operation(seq, 'locator.textContent', { line: 3, startOffsetMs: 40 + seq }),
  ),
]
const failedClick = operation(11, 'locator.click', {
  line: 2,
  effect: 'write',
  startOffsetMs: 60,
  outcome: 'error',
  error: {
    owner: 'script',
    code: 'strict_mode_violation',
    message: "getByRole('button') resolved to 2 elements",
    retryable: false,
  },
})
const background = [
  operation(1, 'events.next', { startOffsetMs: 23, durationMs: 56 }),
  operation(2, 'page.frames', { startOffsetMs: 23 }),
]

function detailWith(run: RunSummary, neo: RunDetail['neo']): RunDetail {
  return { ...runDetail(run), neo }
}

describe("an open run's methods", () => {
  let dom: TestDom
  beforeEach(async () => {
    dom = await installDom()
  })
  afterEach(() => dom.restore())

  const show = (run: RunSummary, detail: RunDetail | null) =>
    dom.render(
      <RunBlock
        run={run}
        formatted={formatCode(run.code)}
        detail={detail}
        selected
        onOpenFull={() => {}}
      />,
    )
  const methods = () =>
    [...dom.container.querySelectorAll('button[data-operation]')].map((row) =>
      row.getAttribute('aria-label'),
    )
  const button = (text: RegExp) =>
    [...dom.container.querySelectorAll('button')].find((candidate) =>
      text.test(candidate.textContent ?? ''),
    ) as Element

  it('puts each method on its line, folds a long loop, and keeps Neo’s own work apart', async () => {
    const run = runSummary(3, { code })
    await show(
      run,
      detailWith(run, record([...background, fill, failedClick, ...loop], 100)),
    )
    expect(methods()).toEqual([
      'locator.fill on line 1',
      'locator.click on line 2',
      'locator.count on line 3',
      // Further calls on the same line have rows of their own, without the code.
      'locator.textContent',
      'locator.textContent',
    ])
    expect(dom.container.textContent).toContain(
      '2 more calls on this line: show',
    )
    await dom.interact(() => fire(button(/more calls on this line/), 'click'))
    expect(methods()).toHaveLength(7)

    // The waterfall bar sits inside the run's 100 ms: fill from +24 ms for 9 ms.
    const bar = dom.container
      .querySelector('[data-operation="5"] [data-bar="waterfall"]')
      ?.getAttribute('style')
    expect(bar).toBe('left:min(24%, calc(100% - 2px));width:max(2px, 9%)')
    expect(dom.container.textContent).toContain('+24 ms')

    const group = button(/not on a line/)
    expect(group.textContent).toContain('2 operations not on a line')
    expect(group.textContent).toContain('events.next, page.frames')
    await dom.interact(() => fire(group, 'click'))
    expect(methods()).toContain('events.next')
  })

  it('folds a line only past four calls: one folded call would save nothing', async () => {
    const calls = (count: number) =>
      Array.from({ length: count }, (_, i) =>
        operation(20 + i, 'locator.textContent', {
          line: 3,
          startOffsetMs: 40 + i,
        }),
      )
    const run = runSummary(3, { code })
    await show(run, detailWith(run, record(calls(4), 100)))
    expect(methods()).toHaveLength(4)
    expect(dom.container.textContent).not.toContain('more call')
    await show(run, detailWith(run, record(calls(5), 100)))
    expect(methods()).toHaveLength(3)
    expect(dom.container.textContent).toContain(
      '2 more calls on this line: show',
    )
  })

  it("opens a method's arguments, result and outcome, saying when Neo truncated them", async () => {
    const run = runSummary(3, { code })
    await show(run, detailWith(run, record([fill, failedClick], 100)))
    const row = dom.container.querySelector('[data-operation="5"]') as Element
    await dom.interact(() => fire(row, 'click'))
    expect(row.getAttribute('aria-expanded')).toBe('true')
    const text = dom.container.textContent ?? ''
    expect(text).toContain("target  locator('aria-ref=e5')")
    expect(text).toContain('value   "Grace Hopper"')
    expect(text).toContain('timeout 5000')
    expect(text).toContain(
      "Truncated: Neo keeps an operation's arguments up to 4 KiB",
    )
    expect(text).toContain(
      "Truncated: Neo keeps an operation's result up to 2 KiB",
    )
    expect(text).toContain('ok, delivery done')

    await dom.interact(() =>
      fire(
        dom.container.querySelector('[data-operation="11"]') as Element,
        'click',
      ),
    )
    const error = dom.container.textContent ?? ''
    expect(error).toContain('script / strict_mode_violation')
    expect(error).toContain("getByRole('button') resolved to 2 elements")
    // One method open at a time.
    expect(row.getAttribute('aria-expanded')).toBe('false')
  })

  it('lays a one-line minified script out by statement, each with its own methods', async () => {
    // What agents send: everything on line 1. Neo's sites point at each method's name.
    const minified =
      "const p=await context.newPage();await p.goto('https://example.com');const t=await p.title();return t;"
    const at = (name: string) => ({
      line: 1,
      startOffsetMs: minified.indexOf(name),
      site: { line: 1, column: minified.indexOf(name) + 1 },
    })
    const run = runSummary(1, { code: minified })
    await show(
      run,
      detailWith(
        run,
        record(
          [
            operation(1, 'context.newPage', at('newPage')),
            operation(2, 'page.goto', at('goto')),
            operation(3, 'page.title', at('title')),
          ],
          100,
        ),
      ),
    )
    expect(methods()).toEqual([
      'context.newPage on line 1',
      'page.goto on line 2',
      'page.title on line 3',
    ])
    const text = dom.container.textContent ?? ''
    expect(text).toContain('2await p.goto(')
    expect(text).toContain('4return t;')
  })

  it("shows the code alone when Neo's record is missing or the run is still running", async () => {
    const ended = runSummary(3, { code })
    await show(ended, detailWith(ended, null))
    expect(methods()).toEqual([])
    expect(dom.container.textContent).toContain(
      'No record from Neo for this run',
    )
    expect(dom.container.textContent).toContain("await page.getByRef('e5')")

    const running = runSummary(4, { code, roundTripMs: null })
    await show(running, detailWith(running, null))
    expect(dom.container.textContent).toContain(
      'Methods appear when the run ends',
    )
  })
})
