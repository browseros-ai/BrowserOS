import { afterEach, describe, expect, it, setSystemTime } from 'bun:test'
import type { MetricsView, RunSummary } from '../shared/api'
import { installDom } from './test-support/dom'
import { runSummary } from './test-support/runs'
import { metricsView, runTrace } from './test-support/telemetry'
import {
  TRACE_WAIT_MS,
  traceMayGrow,
  useMetrics,
  useRunTrace,
} from './use-telemetry'

describe("a run's trace, read as spans arrive", () => {
  it('may grow while the run runs or just ended, and not once complete or long over', () => {
    const now = Date.now()
    const ended = (ago: number): RunSummary => ({
      ...runSummary(1),
      answeredAt: new Date(now - ago).toISOString(),
    })
    expect(traceMayGrow(runSummary(1, { roundTripMs: null }), null, now)).toBe(
      true,
    )
    expect(traceMayGrow(ended(1_000), null, now)).toBe(true)
    expect(traceMayGrow(ended(1_000), runTrace(), now)).toBe(false)
    expect(
      traceMayGrow(ended(1_000), runTrace('neo-3', { complete: false }), now),
    ).toBe(true)
    expect(traceMayGrow(ended(TRACE_WAIT_MS + 1), null, now)).toBe(false)
  })

  it('reads again on each health change until the trace is complete, then stops', async () => {
    const run: RunSummary = {
      ...runSummary(51),
      answeredAt: new Date().toISOString(),
    }
    const traces = new Map([[run.id, runTrace('neo-3', { complete: false })]])
    const dom = await installDom(new Map(), { traces })
    function Spans({ rev }: { rev: number }) {
      const trace = useRunTrace(run, true, rev)
      return (
        <p>
          {trace === undefined
            ? 'reading'
            : trace
              ? `${trace.spans.length}`
              : 'none'}
        </p>
      )
    }
    await dom.render(<Spans rev={1} />)
    expect(dom.container.textContent).toBe('8')
    traces.set(run.id, runTrace('neo-3'))
    await dom.render(<Spans rev={2} />)
    expect(dom.container.textContent).toBe('9')
    const reads = dom.fetched.length
    await dom.render(<Spans rev={3} />)
    await dom.render(<Spans rev={4} />)
    expect(dom.fetched).toHaveLength(reads)
    expect(dom.container.textContent).toBe('9')
    await dom.restore()
  })
})

describe('a trace read before it was whole (review F2, F11)', () => {
  afterEach(() => setSystemTime())

  function Spans({ run, rev }: { run: RunSummary; rev: number }) {
    const trace = useRunTrace(run, true, rev)
    return (
      <p>
        {trace === undefined
          ? 'reading'
          : trace
            ? `${trace.spans.length}`
            : 'none'}
      </p>
    )
  }

  it('reads it again when it is next shown, however late, then stops once whole', async () => {
    const T = Date.parse('2026-11-20T12:00:00.000Z')
    setSystemTime(new Date(T))
    const run: RunSummary = {
      ...runSummary(61),
      answeredAt: new Date(T - 1_000).toISOString(),
    }
    // The block opened just after the run: only the operations had arrived.
    const traces = new Map([[run.id, runTrace('neo-3', { complete: false })]])
    const dom = await installDom(new Map(), { traces })
    await dom.render(<Spans run={run} rev={1} />)
    expect(dom.container.textContent).toBe('8')
    await dom.render(<p>closed</p>)
    // 40 s later, past the run's wait, the block opens again; the viewer has the whole trace.
    traces.set(run.id, runTrace('neo-3'))
    setSystemTime(new Date(T + 40_000))
    await dom.render(<Spans run={run} rev={1} />)
    expect(dom.container.textContent).toBe('9')
    const reads = dom.fetched.length
    await dom.render(<Spans run={run} rev={2} />)
    expect(dom.fetched).toHaveLength(reads)
    await dom.restore()
  })

  it('keeps a trace read after the wait as final: no spans came, and none will', async () => {
    const T = Date.parse('2026-11-20T12:00:00.000Z')
    setSystemTime(new Date(T))
    const run: RunSummary = {
      ...runSummary(63),
      answeredAt: new Date(T - TRACE_WAIT_MS - 1).toISOString(),
    }
    const dom = await installDom(new Map(), {
      traces: new Map([[run.id, null]]),
    })
    await dom.render(<Spans run={run} rev={1} />)
    expect(dom.container.textContent).toBe('none')
    const reads = dom.fetched.length
    await dom.render(<Spans run={run} rev={2} />)
    await dom.render(<p>closed</p>)
    await dom.render(<Spans run={run} rev={3} />)
    expect(dom.fetched).toHaveLength(reads)
    await dom.restore()
  })

  it('reads a lost run once, not on every health change', async () => {
    // The viewer stopped before its reply: no answer time, and its spans went with the viewer.
    const run = runSummary(62, { roundTripMs: null, status: 'lost' })
    expect(run.answeredAt).toBeNull()
    expect(traceMayGrow(run, null, Date.now())).toBe(false)
    const dom = await installDom(new Map(), {
      traces: new Map([[run.id, null]]),
    })
    await dom.render(<Spans run={run} rev={1} />)
    expect(dom.container.textContent).toBe('none')
    const reads = dom.fetched.length
    await dom.render(<Spans run={run} rev={2} />)
    await dom.render(<Spans run={run} rev={3} />)
    expect(dom.fetched).toHaveLength(reads)
    await dom.restore()
  })
})

describe("the Health view's metrics", () => {
  it('reads again when health moves, and an older read that answers late changes nothing', async () => {
    const dom = await installDom()
    const held: Array<(view: MetricsView) => void> = []
    globalThis.fetch = (async (_input: string | URL) =>
      Response.json({
        metrics: await new Promise<MetricsView>((resolve) =>
          held.push(resolve),
        ),
      })) as typeof fetch
    const view = (calls: number): MetricsView => {
      const shape = metricsView()
      const [click] = shape.operations
      if (click) click.total = calls
      return shape
    }
    function Calls({ rev }: { rev: number }) {
      const metrics = useMetrics(true, rev)
      return (
        <p>
          {metrics === undefined
            ? 'reading'
            : (metrics?.operations[0]?.total ?? 'none')}
        </p>
      )
    }
    await dom.render(<Calls rev={1} />)
    expect(dom.container.textContent).toBe('reading')
    await dom.render(<Calls rev={2} />)
    expect(held).toHaveLength(2)
    // The newer read answers first, then the older one.
    await dom.interact(() => held[1]?.(view(22)))
    expect(dom.container.textContent).toBe('22')
    await dom.interact(() => held[0]?.(view(21)))
    expect(dom.container.textContent).toBe('22')
    await dom.restore()
  })
})
