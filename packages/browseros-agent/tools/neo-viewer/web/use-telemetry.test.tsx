import { describe, expect, it } from 'bun:test'
import type { RunSummary } from '../shared/api'
import { installDom } from './test-support/dom'
import { runSummary } from './test-support/runs'
import { runTrace } from './test-support/telemetry'
import { TRACE_WAIT_MS, traceMayGrow, useRunTrace } from './use-telemetry'

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
