import { afterEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import type { RunSummary, TraceSearchResult, TraceSpan } from '../../shared/api'
import { fire, installDom, type TestDom, typeInto } from '../test-support/dom'
import { operation, record } from '../test-support/operations'
import { runDetail, runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
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
      return Response.json({ traces: rows(), truncated: false })
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
