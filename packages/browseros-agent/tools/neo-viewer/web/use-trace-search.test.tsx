import { afterEach, describe, expect, it } from 'bun:test'
import type { RunSummary, TracesResponse } from '../shared/api'
import { installDom, type TestDom } from './test-support/dom'
import { runSummary } from './test-support/runs'
import { useTraceSearch } from './use-trace-search'

function response(durationMs: number): TracesResponse {
  return {
    truncated: false,
    traces: [
      {
        run: runSummary(821),
        traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
        startedAt: 1,
        durationMs,
        spanCount: 1,
        errorCount: 0,
        matchedSpan: null,
      },
    ],
  }
}

function Search({
  runs,
  rev,
  id = '',
}: {
  runs: RunSummary[]
  rev: number
  id?: string
}) {
  const search = useTraceSearch(runs, rev, id, null)
  return (
    <p>
      {search.kind === 'ready'
        ? search.answer.traces[0]?.durationMs
        : search.kind}
    </p>
  )
}

describe('trace search refreshes', () => {
  let dom: TestDom | null = null
  afterEach(async () => {
    await dom?.restore()
    dom = null
  })

  it('retains rows across refreshes and ordinary run arrivals, and drops answers older than the shown revision', async () => {
    dom = await installDom()
    const held: Array<(answer: TracesResponse) => void> = []
    globalThis.fetch = Object.assign(
      async () =>
        Response.json(
          await new Promise<TracesResponse>((resolve) => held.push(resolve)),
        ),
      globalThis.fetch,
    )
    const runs = [runSummary(821)]
    await dom.render(<Search runs={runs} rev={1} />)
    await dom.interact(() => held[0]?.(response(10)))
    await dom.render(<Search runs={runs} rev={2} />)
    expect(dom.container.textContent).toBe('10')
    await dom.render(<Search runs={[...runs, runSummary(822)]} rev={3} />)
    expect(dom.container.textContent).toBe('10')
    await dom.interact(() => held[2]?.(response(30)))
    expect(dom.container.textContent).toBe('30')
    await dom.interact(() => held[1]?.(response(20)))
    expect(dom.container.textContent).toBe('30')
  })

  it('fences a changed query and Clear from pending replies', async () => {
    dom = await installDom()
    const held: Array<(answer: TracesResponse) => void> = []
    globalThis.fetch = Object.assign(
      async () =>
        Response.json(
          await new Promise<TracesResponse>((resolve) => held.push(resolve)),
        ),
      globalThis.fetch,
    )
    const runs = [runSummary(821)]
    await dom.render(<Search runs={runs} rev={1} />)
    await dom.render(<Search runs={runs} rev={1} id="4bf92" />)
    await dom.interact(() => held[1]?.(response(20)))
    await dom.interact(() => held[0]?.(response(10)))
    expect(dom.container.textContent).toBe('20')
    await dom.render(<Search runs={runs} rev={2} id="4bf92" />)
    await dom.render(<Search runs={[]} rev={3} id="4bf92" />)
    expect(dom.container.textContent).toBe('loading')
    await dom.interact(() => held[2]?.(response(30)))
    expect(dom.container.textContent).toBe('loading')
    await dom.interact(() => held[3]?.({ traces: [], truncated: false }))
    expect(dom.container.textContent).toBe('')
  })
})
