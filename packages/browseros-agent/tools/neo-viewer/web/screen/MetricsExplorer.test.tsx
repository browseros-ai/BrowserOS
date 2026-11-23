import { afterEach, describe, expect, it } from 'bun:test'
import { metricsView as buildMetricsView } from '../../server/telemetry/metrics-view'
import type { MetricsView, TelemetryHealth } from '../../shared/api'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { health, metricsView, quantiles } from '../test-support/telemetry'
import type { ExplorerSpanFilter } from './explorer-filter'
import { MetricsExplorer } from './MetricsExplorer'

let dom: TestDom | undefined
afterEach(async () => {
  await dom?.restore()
  dom = undefined
})

async function show(
  metrics: () => MetricsView | null,
  onFindTraces = (_filter: ExplorerSpanFilter) => {},
  shown: TelemetryHealth | null = health(),
) {
  dom = await installDom(new Map(), { metrics })
  await dom.render(
    <MetricsExplorer health={shown} onFindTraces={onFindTraces} />,
  )
  return dom
}

function section(container: HTMLElement, name: string): Element {
  const found = container.querySelector(`section[aria-label="${name}"]`)
  if (!found) throw new Error(`Missing ${name} section`)
  return found
}

describe('Metrics explorer', () => {
  it('shows instance totals, disjoint latency counts and outcomes from the active source', async () => {
    const page = await show(metricsView)
    const totals = page.container.querySelector('[aria-label="Metric totals"]')
    expect(totals?.textContent).toContain('Operations41Across 3 methods')
    expect(totals?.textContent).toContain(
      'Over 500 ms1135% of measured operations',
    )
    expect(totals?.textContent).toContain(
      'Failed operations11Across all duration ranges',
    )
    expect(totals?.textContent).toContain(
      'Completed runs1514 succeeded · 1 in flight',
    )
    const graph = section(page.container, 'Operation duration')
    expect(
      [...graph.querySelectorAll('[data-duration-range]')].map(
        (row) => row.lastElementChild?.textContent,
      ),
    ).toEqual(['0', '0', '20', '11', '0'])
    expect(
      section(page.container, 'Outcomes by operation').textContent,
    ).toContain('locator.click10 non-failed · 11 failed')
    expect(page.container.textContent).toContain('Host API scrape')
    expect(page.fetched).toEqual(['/api/metrics'])
  })

  it('sends exact bucket bounds to Traces and keeps bucket failures unknown', async () => {
    const filters: ExplorerSpanFilter[] = []
    const page = await show(
      () => ({
        ...metricsView(),
        operations: [
          {
            op: 'locator.click',
            total: 5,
            errors: 4,
            duration: quantiles([0.5, 0.6, 1, 1.25, 12], null, null),
          },
        ],
      }),
      (filter) => {
        filters.push(filter)
      },
    )
    const slow = section(page.container, 'Slow operations')
    const rows = [...slow.querySelectorAll('tbody tr')]
    expect(rows.map((row) => row.children[2]?.textContent)).toEqual([
      '2',
      '1',
      '1',
    ])
    expect(rows.map((row) => row.children[3]?.textContent)).toEqual([
      'Unknown',
      'Unknown',
      'Unknown',
    ])
    const first = rows[0]?.querySelector('button')
    const last = rows.at(-1)?.querySelector('button')
    if (!first || !last) throw new Error('Missing trace drilldown actions')
    await page.interact(() => {
      fire(first, 'click')
      fire(last, 'click')
    })
    expect(filters).toEqual([
      { method: 'locator.click', minMs: 500, maxMs: 1000 },
      { method: 'locator.click', minMs: 10000, maxMs: null },
    ])
  })

  it('shows unknown aggregate ranges while retaining exact custom-bound trace actions', async () => {
    const page = await show(() => ({
      ...metricsView(),
      source: 'push',
      operations: [
        {
          op: 'page.goto',
          total: 5,
          errors: 1,
          duration: {
            ...quantiles([], null, null),
            count: 5,
            buckets: [
              { le: 0.8, count: 3 },
              { le: null, count: 5 },
            ],
          },
        },
      ],
    }))
    expect(page.container.textContent).toContain('OTLP push')
    expect(
      page.container.querySelector('[aria-label="Metric totals"]')?.textContent,
    ).toContain('Over 500 ms—Required histogram bound unavailable')
    expect(section(page.container, 'Operation duration').textContent).toContain(
      'Unknown ranges require a bucket bound Neo did not supply.',
    )
    expect(section(page.container, 'Slow operations').textContent).toContain(
      'Over 800 ms',
    )
    expect(section(page.container, 'Slow operations').textContent).toContain(
      'Only known slow intervals are listed.',
    )
  })

  it('shows measured bucket failures separately from the method failure total', async () => {
    const page = await show(() => {
      const view = metricsView()
      view.operations = [
        {
          op: 'locator.click',
          total: 3,
          errors: 2,
          duration: quantiles([0.02, 0.6, 0.7], null, null),
        },
      ]
      view.families.push({
        name: 'neo_operation_duration_seconds',
        kind: 'histogram',
        help: null,
        series: [
          {
            labels: { neo_op_name: 'locator.click', neo_op_outcome: 'error' },
            value: 2,
            buckets: quantiles([0.02, 0.7], null, null).buckets,
          },
          {
            labels: { neo_op_name: 'locator.click', neo_op_outcome: 'ok' },
            value: 1,
            buckets: quantiles([0.6], null, null).buckets,
          },
        ],
      })
      return view
    })
    const row = section(page.container, 'Slow operations').querySelector(
      'tbody tr',
    )
    expect(row?.children[2]?.textContent).toBe('2')
    expect(row?.children[3]?.textContent).toBe('1')
    expect(
      section(page.container, 'Outcomes by operation').textContent,
    ).toContain('2 failed')
  })

  it('renders exact common-bound drilldowns and unknown ranges for mixed outcome bounds', async () => {
    const filters: ExplorerSpanFilter[] = []
    const page = await show(
      () =>
        buildMetricsView(
          [
            {
              name: 'neo_operations_total',
              kind: 'counter',
              help: null,
              series: [
                {
                  labels: {
                    neo_op_name: 'locator.click',
                    neo_op_outcome: 'ok',
                  },
                  value: 5,
                },
                {
                  labels: {
                    neo_op_name: 'locator.click',
                    neo_op_outcome: 'error',
                  },
                  value: 3,
                },
              ],
            },
            {
              name: 'neo_operation_duration_seconds',
              kind: 'histogram',
              help: null,
              series: [
                {
                  labels: {
                    neo_op_name: 'locator.click',
                    neo_op_outcome: 'ok',
                  },
                  value: 5,
                  buckets: [
                    { le: 0.5, count: 1 },
                    { le: 1, count: 3 },
                    { le: 2.5, count: 4 },
                    { le: null, count: 5 },
                  ],
                },
                {
                  labels: {
                    neo_op_name: 'locator.click',
                    neo_op_outcome: 'error',
                  },
                  value: 3,
                  buckets: [
                    { le: 0.5, count: 1 },
                    { le: 2.5, count: 2 },
                    { le: null, count: 3 },
                  ],
                },
              ],
            },
          ],
          'push',
          1,
        ),
      (filter) => filters.push(filter),
    )
    expect(
      page.container.querySelector('[aria-label="Metric totals"]')?.textContent,
    ).toContain('Over 500 ms675% of measured operations')
    const graph = section(page.container, 'Operation duration')
    expect(
      graph.querySelector('[data-duration-range=">500 ms to 1 s"]')
        ?.textContent,
    ).toContain('Unknown')
    const rows = [
      ...section(page.container, 'Slow operations').querySelectorAll(
        'tbody tr',
      ),
    ]
    expect(rows.map((row) => row.children[2]?.textContent)).toEqual(['4', '2'])
    expect(rows.map((row) => row.children[3]?.textContent)).toEqual(['1', '1'])
    const action = rows[0]?.querySelector('button')
    if (!action) throw new Error('Missing common-bound trace action')
    await page.interact(() => fire(action, 'click'))
    expect(filters).toEqual([
      { method: 'locator.click', minMs: 500, maxMs: 2500 },
    ])
  })

  it('uses explicit error outcomes for the failure KPI and keeps cancellations separate', async () => {
    const page = await show(() => ({
      ...metricsView(),
      operations: [
        {
          op: 'events.next',
          total: 6,
          // Health attribution includes these cancellations; the Metrics chart does not.
          errors: 5,
          duration: quantiles([], null, null),
        },
      ],
      families: [
        {
          name: 'neo_operations_total',
          kind: 'counter',
          help: null,
          series: [
            {
              labels: { neo_op_name: 'events.next', neo_op_outcome: 'ok' },
              value: 1,
            },
            {
              labels: { neo_op_name: 'events.next', neo_op_outcome: 'error' },
              value: 2,
            },
            {
              labels: {
                neo_op_name: 'events.next',
                neo_op_outcome: 'cancelled',
              },
              value: 3,
            },
          ],
        },
      ],
    }))
    expect(
      page.container.querySelector('[aria-label="Metric totals"]')?.textContent,
    ).toContain('Failed operations2Across all duration ranges')
    expect(
      section(page.container, 'Outcomes by operation').textContent,
    ).toContain('1 succeeded · 2 failed · 3 cancelled')
  })

  it('refreshes on health revisions without switching to since-Clear totals', async () => {
    let calls = 41
    const read = () => ({
      ...metricsView(),
      operations: [
        {
          op: 'page.goto',
          total: calls++,
          errors: 0,
          duration: quantiles([], null, null),
        },
      ],
    })
    const page = await show(read)
    expect(
      page.container.querySelector('[aria-label="Metric totals"]')?.textContent,
    ).toContain('Operations41')
    await page.render(
      <MetricsExplorer
        health={health({
          rev: 2,
          sinceClear: {
            runs: 0,
            runErrors: 0,
            ops: 0,
            opErrors: 0,
            crashes: 0,
          },
        })}
        onFindTraces={() => {}}
      />,
    )
    expect(
      page.container.querySelector('[aria-label="Metric totals"]')?.textContent,
    ).toContain('Operations42')
    expect(page.fetched).toEqual(['/api/metrics', '/api/metrics'])
  })

  it('distinguishes no source, no operations, and calls without measured durations', async () => {
    let view: MetricsView | null = null
    const page = await show(() => view, undefined, null)
    expect(page.container.textContent).toContain('No live metrics.')
    expect(
      page.container.querySelector('[aria-label="Metric totals"]'),
    ).toBeNull()
    view = { ...metricsView(), operations: [] }
    await page.render(
      <MetricsExplorer health={health({ rev: 2 })} onFindTraces={() => {}} />,
    )
    expect(page.container.textContent).toContain(
      'No operation has settled since Neo started.',
    )
    expect(page.container.textContent).toContain(
      'No duration samples to investigate yet.',
    )
    view = {
      ...metricsView(),
      operations: [
        {
          op: 'page.title',
          total: 2,
          errors: 0,
          duration: quantiles([], null, null),
        },
      ],
    }
    await page.render(
      <MetricsExplorer health={health({ rev: 3 })} onFindTraces={() => {}} />,
    )
    expect(page.container.textContent).toContain('page.title2 non-failed')
    expect(page.container.textContent).toContain(
      'No operation duration samples have been recorded.',
    )
    expect(
      page.container.querySelector('[aria-label="Metric totals"]')?.textContent,
    ).toContain('Over 500 ms—No duration samples')
  })
})
