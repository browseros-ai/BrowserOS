import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import type { RunDetail, RunSummary, RunTrace } from '../../shared/api'
import { formatCode } from '../../shared/code-format'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { operation, record } from '../test-support/operations'
import { runDetail, runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
import { SPANS_NOTICE } from './operation-output'
import type { OperationSelection } from './operations'
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
    operation(seq, 'locator.textContent', {
      site: { line: 3, column: 72 },
      startOffsetMs: 40 + seq,
    }),
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

/** Every selection the block made, newest last: what the screen would show on the right. */
let selections: Array<OperationSelection | null> = []

/** The block with its selection held as the screen holds it. */
function Block({ run, detail }: { run: RunSummary; detail: RunDetail | null }) {
  const [selection, setSelection] = useState<OperationSelection | null>(null)
  return (
    <RunBlock
      run={run}
      formatted={formatCode(run.code)}
      detail={detail}
      selected
      selectedOperationId={selection?.operationId ?? null}
      onSelect={(next) => {
        selections.push(next)
        setSelection(next)
      }}
      onOpenFull={() => {}}
    />
  )
}

describe("an open run's methods", () => {
  let dom: TestDom
  beforeEach(async () => {
    dom = await installDom()
    selections = []
  })
  afterEach(() => dom.restore())

  const show = (run: RunSummary, detail: RunDetail | null) =>
    dom.render(<Block run={run} detail={detail} />)
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
      // Further calls on the same line have rows of their own, without the code; a method the
      // line calls again says which call it is.
      'locator.textContent',
      'locator.textContent, 2nd call',
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
    expect(group.textContent).toContain(
      'Neo internal: 2 operations not on a line',
    )
    expect(group.textContent).toContain('events.next, page.frames')
    await dom.interact(() => fire(group, 'click'))
    expect(methods()).toContain('events.next, Neo internal')
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

  it('reveals a selected folded call and a nested internal call when its record arrives', async () => {
    const run = runSummary(36, { code })
    const nested = operation(12, 'frame.waitForLoadState', {
      parentId: fill.id,
    })
    const render = (detail: RunDetail | null, selectedOperationId: string) => (
      <RunBlock
        run={run}
        formatted={formatCode(run.code)}
        detail={detail}
        selected
        selectedOperationId={selectedOperationId}
        onSelect={() => {}}
        onOpenFull={() => {}}
      />
    )
    const detail = detailWith(run, record([fill, ...loop, nested]))
    await dom.render(render(null, 'neo-run.10'))
    expect(dom.container.querySelector('[data-operation="10"]')).toBeNull()
    await dom.render(render(detail, 'neo-run.10'))
    expect(
      dom.container
        .querySelector('[data-operation="10"]')
        ?.getAttribute('aria-expanded'),
    ).toBe('true')
    expect(dom.container.textContent).not.toContain('more calls on this line')
    await dom.render(render(detail, nested.id))
    expect(
      dom.container
        .querySelector('[data-operation="12"]')
        ?.getAttribute('aria-expanded'),
    ).toBe('true')
    expect(methods()).toContain('frame.waitForLoadState, inside op 5')
    const internal = button(/not on a line/)
    await dom.interact(() => fire(internal, 'click'))
    expect(internal.getAttribute('aria-expanded')).toBe('false')
    expect(dom.container.querySelector('[data-operation="12"]')).toBeNull()
    await dom.render(render(detail, nested.id))
    expect(internal.getAttribute('aria-expanded')).toBe('false')
    await dom.render(render(detail, 'neo-run.10'))
    await dom.render(render(detail, nested.id))
    expect(internal.getAttribute('aria-expanded')).toBe('true')
  })

  it('keeps the rest of a long line folded when its selected call is already visible', async () => {
    const run = runSummary(37, { code })
    await dom.render(
      <RunBlock
        run={run}
        formatted={formatCode(run.code)}
        detail={detailWith(run, record(loop))}
        selected
        selectedOperationId="neo-run.6"
        onSelect={() => {}}
        onOpenFull={() => {}}
      />,
    )
    expect(dom.container.querySelector('[data-operation="10"]')).toBeNull()
    expect(dom.container.textContent).toContain(
      '2 more calls on this line: show',
    )
  })

  it("selects a method: its arguments and outcome open under it, its result's size points right", async () => {
    const run = runSummary(3, { code })
    await show(run, detailWith(run, record([fill, failedClick], 100)))
    const row = dom.container.querySelector('[data-operation="5"]') as Element
    await dom.interact(() => fire(row, 'click'))
    expect(row.getAttribute('aria-expanded')).toBe('true')
    expect(selections.at(-1)).toEqual({
      runId: run.id,
      operationId: 'neo-run.5',
      place: { line: 1, call: 1, calls: 1 },
    })
    const text = dom.container.textContent ?? ''
    expect(text).toContain("target  locator('aria-ref=e5')")
    expect(text).toContain('value   "Grace Hopper"')
    expect(text).toContain('timeout 5000')
    expect(text).toContain(
      "Truncated: Neo kept only the start of this operation's arguments.",
    )
    expect(text).toContain('10 chars, ≈2 tok, shown on the right')
    expect(text).toContain('Truncated: Neo kept only the start of this result.')
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

  it('a line in a loop counts its calls, and each later call says which it is', async () => {
    // The owner's loop: three pages, each opened, loaded and read on lines 2 to 4.
    const loopCode = [
      'for (const url of urls) {',
      '  const p = await context.newPage();',
      '  await p.goto(url);',
      '  values.push(await p.evaluate(() => document.title));',
      '}',
    ].join('\n')
    const calls = [0, 1, 2].flatMap((pass) => [
      operation(10 + pass * 3, 'context.newPage', {
        line: 2,
        startOffsetMs: 100 + pass * 400,
      }),
      operation(11 + pass * 3, 'page.goto', {
        line: 3,
        startOffsetMs: 110 + pass * 400,
      }),
      operation(12 + pass * 3, 'page.evaluate', {
        line: 4,
        startOffsetMs: 300 + pass * 400,
      }),
    ])
    const run = runSummary(8, { code: loopCode })
    await show(run, detailWith(run, record(calls, 1300)))
    expect(methods()).toEqual([
      'context.newPage on line 2',
      'context.newPage, 2nd call',
      'context.newPage, 3rd call',
      'page.goto on line 3',
      'page.goto, 2nd call',
      'page.goto, 3rd call',
      'page.evaluate on line 4',
      'page.evaluate, 2nd call',
      'page.evaluate, 3rd call',
    ])
    const counts = [
      ...dom.container.querySelectorAll('[title^="This line ran"]'),
    ]
    expect(counts.map((badge) => badge.textContent)).toEqual(['×3', '×3', '×3'])
    // Start is the start, measured from the run's start: the header says so, and each offset.
    const start = dom.container.querySelector(
      '[title^="When the call started"]',
    )
    expect(start?.textContent).toContain('Start')
    const second = dom.container.querySelector(
      '[data-operation="13"]',
    ) as Element
    expect(second.textContent).toContain('+500 ms')
    expect(
      second.querySelector('[title^="Started"]')?.getAttribute('title'),
    ).toBe('Started 500 ms after the run started.')
    await dom.interact(() => fire(second, 'click'))
    expect(selections.at(-1)?.place).toEqual({ line: 2, call: 2, calls: 3 })
  })

  it('counts the line on its first row even when a later call on it is the one repeated', async () => {
    const lineCode =
      'const titles = await Promise.all((await context.pages()).map((p) => p.title()))'
    const once = operation(3, 'context.pages', {
      site: { line: 1, column: 35 },
    })
    const titles = [4, 5, 6].map((seq) =>
      operation(seq, 'page.title', {
        site: { line: 1, column: 70 },
        startOffsetMs: seq,
      }),
    )
    const run = runSummary(5, { code: lineCode })
    await show(run, detailWith(run, record([once, ...titles], 100)))
    const counts = [
      ...dom.container.querySelectorAll('[title^="This line ran"]'),
    ]
    expect(counts.map((badge) => badge.textContent)).toEqual(['×3'])
    expect(methods()).toEqual([
      'context.pages on line 1',
      'page.title',
      'page.title, 2nd call',
      'page.title, 3rd call',
    ])
  })

  it("labels Neo's own bookkeeping on a line, and the Model read row selects the reply again", async () => {
    const snapshotCode = 'return await page.ariaSnapshot({ text: true })'
    const operations = [
      operation(4, 'page.ariaSnapshot', { line: 1, startOffsetMs: 20 }),
      operation(5, 'page.snapshotPending', { line: 1, startOffsetMs: 80 }),
    ]
    const run = runSummary(9, { code: snapshotCode, reply: '"x"' })
    await show(run, detailWith(run, record(operations, 100)))
    expect(methods()).toEqual([
      'page.ariaSnapshot on line 1',
      'page.snapshotPending, Neo internal',
    ])
    await dom.interact(() =>
      fire(
        dom.container.querySelector('[data-operation="4"]') as Element,
        'click',
      ),
    )
    await dom.interact(() => fire(button(/^Model read$/), 'click'))
    // The reply's own line selects it too, beside its Open full.
    await dom.interact(() =>
      fire(
        dom.container.querySelector('[data-operation="4"]') as Element,
        'click',
      ),
    )
    const chip = [
      ...dom.container.querySelectorAll(
        'button[title="Show the reply in the right panel"]',
      ),
    ].at(-1) as Element
    await dom.interact(() => fire(chip, 'click'))
    expect(selections.at(-1)).toBeNull()
    selections.splice(2)
    expect(selections).toEqual([
      {
        runId: run.id,
        operationId: 'neo-run.4',
        place: { line: 1, call: 1, calls: 1 },
      },
      null,
    ])
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

  it('explains why a run has no recorded values while keeping its methods and proxy code visible', async () => {
    const run = runSummary(3, { code })
    const step = operation(5, 'locator.fill', {
      line: 1,
      values: 'off',
      args: null,
      result: null,
    })
    await show(run, detailWith(run, { ...record([step]), values: 'off' }))
    expect(dom.container.textContent).toContain(
      'Neo did not record values for this run (trace.values is off)',
    )
    expect(methods()).toEqual(['locator.fill on line 1'])
    expect(dom.container.textContent).toContain("await page.getByRef('e5')")
    expect(dom.container.textContent).not.toContain('No record from Neo')
  })

  it('marks one operation without hiding an earlier recorded operation in a mixed run', async () => {
    const run = runSummary(3, { code })
    const omitted = operation(12, 'locator.click', {
      line: 2,
      values: 'off',
      args: null,
      result: null,
      outcome: 'error',
      delivery: 'not_sent',
      error: {
        owner: 'script',
        code: 'no_match',
        message: '',
        retryable: true,
      },
    })
    await show(run, detailWith(run, record([fill, omitted])))
    expect(dom.container.textContent).not.toContain(
      'Neo did not record values for this run',
    )
    const row = dom.container.querySelector('[data-operation="12"]')
    if (!row) throw new Error('the omitted operation still has a row')
    await dom.interact(() => fire(row, 'click'))
    expect(dom.container.textContent).toContain(
      'Neo did not record values for this operation (trace.values is off)',
    )
    expect(dom.container.textContent).toContain('script / no_match')
    expect(dom.container.textContent).toContain('error, delivery not_sent')
    expect(dom.container.textContent).not.toContain('No call log recorded.')
    expect(dom.container.textContent).not.toContain('Arguments')

    const earlier = dom.container.querySelector('[data-operation="5"]')
    if (!earlier)
      throw new Error('the earlier recorded operation still has a row')
    await dom.interact(() => fire(earlier, 'click'))
    expect(dom.container.textContent).toContain('value   "Grace Hopper"')
    expect(dom.container.textContent).toContain(
      '10 chars, ≈2 tok, shown on the right',
    )
    expect(dom.container.textContent).not.toContain(
      'Neo did not record values for this operation',
    )
  })
})

describe("an open run's trace row", () => {
  let dom: TestDom
  afterEach(() => dom.restore())
  // Traces read are kept per run (id and send time), so each case has a run of its own.
  let run = runSummary(3, { code: 'return 1' })

  async function block(
    props: Partial<Parameters<typeof RunBlock>[0]>,
    trace: RunTrace | null = runTrace('neo-run'),
    seq = 3,
  ) {
    run = runSummary(seq, { code: 'return 1' })
    dom = await installDom(new Map(), {
      traces: new Map([[run.id, trace]]),
    })
    await dom.render(
      <RunBlock
        run={run}
        formatted={formatCode(run.code)}
        detail={runDetail(run)}
        selected
        selectedOperationId={null}
        onSelect={() => {}}
        onOpenFull={() => {}}
        {...props}
      />,
    )
  }
  const rowOf = (label: string) =>
    [...dom.container.querySelectorAll('div.h-7')].find((row) =>
      row.querySelector('button')?.textContent?.startsWith(label),
    ) as Element | undefined

  it('is not there until Neo’s spans reach the viewer', async () => {
    await block({ traceRev: null }, runTrace('neo-run'), 32)
    expect(rowOf('Trace')).toBeUndefined()
    expect(dom.fetched.some((path) => path.endsWith('/trace'))).toBe(false)
  })

  it("says what the run's trace holds, and shows it on the right when picked", async () => {
    let picked = 0
    await block({ traceRev: 4, onSelectTrace: () => picked++ })
    const row = rowOf('Trace')
    expect(row?.textContent).toBe(
      'Trace4bf92f359 spans, 1 failed, 1.23 sOpen full',
    )
    const open = row?.querySelector('button')
    if (!open) throw new Error('the trace row has its button')
    await dom.interact(() => fire(open, 'click'))
    expect(picked).toBe(1)
  })

  it('marks the trace row while the right panel shows the trace, and Model read no longer', async () => {
    await block({ traceRev: 4, traceShown: true }, runTrace('neo-run'), 33)
    expect(rowOf('Trace')?.className).toContain('bg-[#DCE6FB]')
    expect(rowOf('Model read')?.className).not.toContain('bg-[#DCE6FB]')
    expect(rowOf('Trace')?.textContent).toContain(', shown on the right')
  })

  it('says when a run has no spans, without asking again once none can come', async () => {
    await block({ traceRev: 4 }, null, 31)
    expect(rowOf('Trace')?.textContent).toBe(
      'TraceNo spans from Neo for this run.',
    )
    const reads = dom.fetched.filter((path) => path.endsWith('/trace')).length
    await dom.render(
      <RunBlock
        run={run}
        formatted={formatCode(run.code)}
        detail={runDetail(run)}
        selected
        selectedOperationId={null}
        onSelect={() => {}}
        onOpenFull={() => {}}
        traceRev={5}
      />,
    )
    expect(dom.fetched.filter((path) => path.endsWith('/trace'))).toHaveLength(
      reads,
    )
  })
})

describe('a run whose record came from Neo’s spans', () => {
  let dom: TestDom
  beforeEach(async () => {
    dom = await installDom()
    selections = []
  })
  afterEach(() => dom.restore())

  it('says once that spans carry no arguments or results, and opens a call to its outcome alone', async () => {
    const run = runSummary(3, { code })
    const fromSpans: RunDetail['neo'] = {
      ...record(
        [
          {
            ...fill,
            kind: '',
            effect: '',
            args: null,
            result: null,
            notes: [],
          },
          {
            ...failedClick,
            kind: '',
            effect: '',
            args: null,
            result: null,
            notes: [],
          },
        ],
        100,
      ),
      source: 'spans',
    }
    await dom.render(<Block run={run} detail={detailWith(run, fromSpans)} />)
    const text = dom.container.textContent ?? ''
    expect(text.split(SPANS_NOTICE)).toHaveLength(2)
    const click = dom.container.querySelector(
      'button[data-operation="11"]',
    ) as Element
    await dom.interact(() => fire(click, 'click'))
    const detail = dom.container.textContent ?? ''
    expect(detail).toContain('Outcome')
    expect(detail).toContain(
      "script / strict_mode_violationgetByRole('button') resolved to 2 elements",
    )
    expect(detail).not.toContain('Arguments')
    expect(detail).not.toContain('No call log recorded.')
  })
})
