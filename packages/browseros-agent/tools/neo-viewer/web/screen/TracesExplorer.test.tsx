import { afterEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import type { RunSummary, TraceSearchResult, TraceSpan } from '../../shared/api'
import { fire, installDom, type TestDom, typeInto } from '../test-support/dom'
import { operation, record } from '../test-support/operations'
import { runDetail, runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
import { useRunTrace } from '../use-telemetry'
import type { ExplorerSpanFilter } from './explorer-filter'
import type { OperationSelection } from './operations'
import { TracesExplorer } from './TracesExplorer'

function result(run: RunSummary): TraceSearchResult {
  return {
    run,
    traceId: runTrace().traceId,
    startedAt: Date.parse(run.sentAt),
    durationMs: 1230,
    spanCount: 9,
    errorCount: 1,
    matchedSpan: {
      spanId: 'span-7',
      name: 'locator.click',
      label: 'locator.click',
      durationMs: 508,
    },
  }
}

/** Keep the fixture's run/trace routes and supply the new search boundary separately. */
function searchRoute(rows: () => TraceSearchResult[]): string[] {
  const base = globalThis.fetch
  const paths: string[] = []
  globalThis.fetch = Object.assign(
    async (...args: Parameters<typeof fetch>) => {
      const path = String(args[0])
      if (!path.startsWith('/api/traces')) return base(...args)
      paths.push(path)
      return Response.json({
        traces: rows(),
        truncated: false,
        nextCursor: null,
      })
    },
    base,
  )
  return paths
}

function button(dom: TestDom, text: string): Element {
  const found = [...dom.container.querySelectorAll('button')].find(
    (candidate) => candidate.textContent === text,
  )
  if (!found) throw new Error(`Missing button ${text}`)
  return found
}

describe('the Traces tab', () => {
  let dom: TestDom | null = null
  afterEach(async () => {
    await dom?.restore()
    dom = null
  })

  it('loads older matching traces and keeps the chosen row across background arrivals', async () => {
    const runs = [runSummary(851), runSummary(852), runSummary(853)]
    const newer = runSummary(854)
    dom = await installDom()
    const base = globalThis.fetch
    let arrived = false
    const paths: string[] = []
    globalThis.fetch = Object.assign(
      async (...args: Parameters<typeof fetch>) => {
        const path = String(args[0])
        if (!path.startsWith('/api/traces')) return base(...args)
        paths.push(path)
        const continuation = path.includes('cursor=')
        const page = continuation
          ? arrived
            ? runs.slice(0, 2).reverse()
            : runs.slice(0, 1)
          : arrived
            ? [newer, ...runs.slice(2)]
            : runs.slice(1).reverse()
        return Response.json({
          traces: page.map(result),
          truncated: !continuation,
          nextCursor: continuation ? null : 'older',
        })
      },
      base,
    )
    const render = (rev: number) => (
      <TracesExplorer
        runs={arrived ? [...runs, newer] : runs}
        rev={rev}
        filter={{ method: 'locator.click', minMs: 500, maxMs: 1000 }}
        onClearFilter={() => {}}
        onOpenRun={() => {}}
      />
    )
    await dom.render(render(1))
    const more = button(dom, 'Load more traces')
    await dom.interact(() => fire(more, 'click'))
    expect(dom.container.querySelectorAll('[data-trace-run]')).toHaveLength(3)
    expect(paths.at(-1)).toBe(
      '/api/traces?method=locator.click&minMs=500&maxMs=1000&cursor=older',
    )
    const older = dom.container.querySelector('[data-trace-run="run-851"]')
    if (!older) throw new Error('Missing older trace row')
    await dom.interact(() => fire(older, 'click'))
    arrived = true
    await dom.render(render(2))
    expect(dom.container.querySelectorAll('[data-trace-run]')).toHaveLength(4)
    expect(
      dom.container
        .querySelector('[data-trace-run="run-851"]')
        ?.getAttribute('aria-pressed'),
    ).toBe('true')
    expect(dom.container.textContent).not.toContain('Load more traces')
  })

  it('opens the matching received span, copies its trace ID, and opens its actual operation in Runs', async () => {
    const run = runSummary(811, {
      code: 'await page.locator("button").click()',
    })
    const detail = runDetail(run)
    detail.neo = record([
      operation(7, 'locator.click', { id: 'neo-run.7', line: 1 }),
    ])
    dom = await installDom(new Map([[run.id, detail]]), {
      traces: new Map([[run.id, runTrace('neo-run')]]),
    })
    const paths = searchRoute(() => [result(run)])
    const copied: string[] = []
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          copied.push(text)
        },
      },
    })
    const opened: Array<{
      run: RunSummary
      span: TraceSpan | null
      operation: OperationSelection | null | undefined
    }> = []
    await dom.render(
      <TracesExplorer
        runs={[run]}
        rev={1}
        filter={{ method: 'locator.click', minMs: 500, maxMs: 1000 }}
        onClearFilter={() => {}}
        onOpenRun={(run, span, operation) =>
          opened.push({ run, span, operation })
        }
      />,
    )
    expect(paths[0]).toBe(
      '/api/traces?method=locator.click&minMs=500&maxMs=1000',
    )
    expect(dom.container.textContent).toContain('1 matching trace')
    expect(dom.container.textContent).toContain('508 ms')
    expect(dom.container.textContent).toContain('1.23 s')
    expect(
      dom.container
        .querySelector('[data-span="span-7"]')
        ?.getAttribute('aria-pressed'),
    ).toBe('true')
    expect(
      dom.container.querySelector('[data-attributes]')?.textContent,
    ).toContain('no_match')
    const copy = button(dom, 'Copy trace ID')
    await dom.interact(() => fire(copy, 'click'))
    expect(copied).toEqual([runTrace().traceId])
    const open = button(dom, 'Open in Runs ↗')
    await dom.interact(() => fire(open, 'click'))
    expect(opened[0]?.span?.spanId).toBe('span-7')
    expect(opened[0]?.operation).toEqual({
      runId: run.id,
      operationId: 'neo-run.7',
      place: { line: 1, call: 1, calls: 1 },
    })
  })

  it('combines a partial trace ID with the metric range and clears both filters', async () => {
    const run = runSummary(812)
    dom = await installDom()
    const paths = searchRoute(() => [])
    const runs = [run]
    function Explorer() {
      const [filter, setFilter] = useState<ExplorerSpanFilter | null>({
        method: 'page.goto',
        minMs: 250,
        maxMs: null,
      })
      return (
        <TracesExplorer
          runs={runs}
          rev={1}
          filter={filter}
          onClearFilter={() => setFilter(null)}
          onOpenRun={() => {}}
        />
      )
    }
    await dom.render(<Explorer />)
    const input = dom.container.querySelector<HTMLInputElement>('input')
    if (!input) throw new Error('Missing search input')
    await dom.interact(() => typeInto(input, '4BF92'))
    expect(paths.at(-1)).toBe(
      '/api/traces?traceId=4BF92&method=page.goto&minMs=250',
    )
    expect(dom.container.textContent).toContain(
      'No received spans match these filters.',
    )
    const clear = button(dom, 'Clear filters')
    await dom.interact(() => fire(clear, 'click'))
    expect(input.value).toBe('')
    expect(paths.at(-1)).toBe('/api/traces')
    expect(dom.container.textContent).toContain(
      'No traces received for the recorded runs yet.',
    )
    expect(dom.container.textContent).not.toContain('Duration')
  })

  it('bypasses a settled empty trace captured by another view before a late batch', async () => {
    const run = {
      ...runSummary(820),
      answeredAt: new Date(Date.now() - 60_000).toISOString(),
    }
    const traces = new Map<string, ReturnType<typeof runTrace> | null>([
      [run.id, null],
    ])
    dom = await installDom(new Map(), { traces })
    function RunsTrace() {
      const trace = useRunTrace(run, true, 1)
      return <p>{trace === null ? 'No spans' : 'Reading'}</p>
    }
    await dom.render(<RunsTrace />)
    expect(dom.container.textContent).toBe('No spans')
    traces.set(run.id, runTrace('neo-run'))
    searchRoute(() => [result(run)])
    await dom.render(
      <TracesExplorer
        runs={[run]}
        rev={2}
        filter={null}
        onClearFilter={() => {}}
        onOpenRun={() => {}}
      />,
    )
    expect(dom.container.querySelectorAll('[data-span]')).toHaveLength(9)
    expect(
      dom.container
        .querySelector('[data-span="span-7"]')
        ?.getAttribute('aria-pressed'),
    ).toBe('true')
  })

  it('refreshes root-first and retried spans without changing the picked span', async () => {
    const run = runSummary(815)
    const complete = runTrace('neo-run')
    const traces = new Map([
      [run.id, { ...complete, spans: complete.spans.slice(0, 1) }],
    ])
    dom = await installDom(new Map(), { traces })
    searchRoute(() => [{ ...result(run), matchedSpan: null }])
    const runs = [run]
    const explorer = (rev: number) => (
      <TracesExplorer
        runs={runs}
        rev={rev}
        filter={null}
        onClearFilter={() => {}}
        onOpenRun={() => {}}
      />
    )
    await dom.render(explorer(1))
    expect(dom.container.querySelectorAll('[data-span]')).toHaveLength(1)
    traces.set(run.id, complete)
    await dom.render(explorer(2))
    const picked = dom.container.querySelector('[data-span="span-7"]')
    if (!picked) throw new Error('Missing late operation span')
    await dom.interact(() => fire(picked, 'click'))
    const waterfall = dom.container.querySelector('[data-waterfall]')
    traces.set(run.id, {
      ...complete,
      spans: complete.spans.map((span) =>
        span.spanId === 'span-7'
          ? {
              ...span,
              durationMs: 600,
              attributes: { ...span.attributes, 'neo.retry': true },
            }
          : span,
      ),
    })
    await dom.render(explorer(3))
    expect(dom.container.querySelector('[data-waterfall]')).toBe(waterfall)
    expect(
      dom.container
        .querySelector('[data-span="span-7"]')
        ?.getAttribute('aria-pressed'),
    ).toBe('true')
    expect(
      dom.container.querySelector('[data-attributes]')?.textContent,
    ).toContain('neo.retry')
    expect(
      dom.container.querySelector('[data-span="span-7"]')?.textContent,
    ).toContain('600 ms')
  })

  it.each(['trace', 'record'] as const)(
    'waits for the %s read before opening the matching operation',
    async (heldRead) => {
      const run = runSummary(heldRead === 'trace' ? 816 : 817, {
        code: 'await page.locator("button").click()',
      })
      const detail = runDetail(run)
      detail.neo = record([
        operation(7, 'locator.click', { id: 'neo-run.7', line: 1 }),
      ])
      dom = await installDom(new Map([[run.id, detail]]), {
        traces: new Map([[run.id, runTrace('neo-run')]]),
      })
      searchRoute(() => [result(run)])
      const base = globalThis.fetch
      const held: Array<(response: Response) => void> = []
      const path = `/api/runs/${run.id}${heldRead === 'trace' ? '/trace' : ''}`
      globalThis.fetch = Object.assign(
        async (...args: Parameters<typeof fetch>) => {
          if (String(args[0]) !== path) return base(...args)
          return new Promise<Response>((resolve) => held.push(resolve))
        },
        base,
      )
      const opened: Array<OperationSelection | null | undefined> = []
      await dom.render(
        <TracesExplorer
          runs={[run]}
          rev={1}
          filter={null}
          onClearFilter={() => {}}
          onOpenRun={(_run, _span, operation) => opened.push(operation)}
        />,
      )
      const open = button(dom, 'Open in Runs ↗')
      expect(open.hasAttribute('disabled')).toBe(true)
      await dom.interact(() => fire(open, 'click'))
      expect(opened).toEqual([])
      await dom.interact(() =>
        held[0]?.(
          Response.json(
            heldRead === 'trace'
              ? { trace: runTrace('neo-run') }
              : { run: detail },
          ),
        ),
      )
      expect(open.hasAttribute('disabled')).toBe(false)
      await dom.interact(() => fire(open, 'click'))
      expect(opened[0]?.operationId).toBe('neo-run.7')
    },
  )

  it.each(['absent', 'failed'] as const)(
    'still opens the run and span when its operation record is %s',
    async (response) => {
      const run = runSummary(response === 'absent' ? 818 : 819)
      dom = await installDom(
        response === 'absent' ? new Map([[run.id, runDetail(run)]]) : new Map(),
        { traces: new Map([[run.id, runTrace('neo-run')]]) },
      )
      searchRoute(() => [result(run)])
      const opened: Array<{
        span: TraceSpan | null
        operation: OperationSelection | null | undefined
      }> = []
      await dom.render(
        <TracesExplorer
          runs={[run]}
          rev={1}
          filter={null}
          onClearFilter={() => {}}
          onOpenRun={(_run, span, operation) =>
            opened.push({ span, operation })
          }
        />,
      )
      const open = button(dom, 'Open in Runs ↗')
      expect(open.hasAttribute('disabled')).toBe(false)
      await dom.interact(() => fire(open, 'click'))
      expect(opened[0]?.span?.spanId).toBe('span-7')
      expect(opened[0]?.operation).toBeNull()
    },
  )

  it('still opens the run when its trace read fails', async () => {
    const run = runSummary(823)
    dom = await installDom(new Map([[run.id, runDetail(run)]]))
    searchRoute(() => [result(run)])
    const opened: Array<TraceSpan | null> = []
    await dom.render(
      <TracesExplorer
        runs={[run]}
        rev={1}
        filter={null}
        onClearFilter={() => {}}
        onOpenRun={(_run, span) => opened.push(span)}
      />,
    )
    const open = button(dom, 'Open in Runs ↗')
    expect(open.hasAttribute('disabled')).toBe(false)
    await dom.interact(() => fire(open, 'click'))
    expect(opened).toEqual([null])
    expect(dom.container.textContent).toContain('Could not load spans.')
  })

  it('keeps shared trace IDs as separate runs, refreshes partial spans, and removes a cleared selection', async () => {
    const run = { ...runSummary(813), answeredAt: new Date().toISOString() }
    const other = runSummary(814)
    const traces = new Map([[run.id, runTrace('neo-run', { complete: false })]])
    dom = await installDom(new Map(), { traces })
    let rows = [result(run), result(other)]
    searchRoute(() => rows)
    const render = (runs: RunSummary[], rev: number) => (
      <TracesExplorer
        runs={runs}
        rev={rev}
        filter={null}
        onClearFilter={() => {}}
        onOpenRun={() => {}}
      />
    )
    const runs = [run, other]
    await dom.render(render(runs, 1))
    expect(dom.container.querySelectorAll('[data-trace-run]')).toHaveLength(2)
    expect(dom.container.textContent).toContain('waiting for neo.run')
    traces.set(run.id, runTrace('neo-run'))
    await dom.render(render(runs, 2))
    expect(dom.container.querySelector('[data-span="span-run"]')).not.toBeNull()
    expect(dom.container.textContent).not.toContain('waiting for neo.run')
    rows = [result(other), result(run)]
    await dom.render(render(runs, 3))
    expect(
      dom.container
        .querySelector(`[data-trace-run="${run.id}"]`)
        ?.getAttribute('aria-pressed'),
    ).toBe('true')
    rows = []
    await dom.render(render([], 4))
    expect(dom.container.querySelectorAll('[data-trace-run]')).toHaveLength(0)
    expect(dom.container.querySelector('[data-waterfall]')).toBeNull()
    expect(dom.container.textContent).not.toContain('Open in Runs')
  })
})
