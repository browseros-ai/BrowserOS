import { afterEach, describe, expect, it } from 'bun:test'
import type { RunSummary, TracesResponse } from '../shared/api'
import { fire, installDom, type TestDom } from './test-support/dom'
import { runSummary } from './test-support/runs'
import { useTraceSearch } from './use-trace-search'

function response(durationMs: number): TracesResponse {
  return {
    truncated: false,
    nextCursor: null,
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

function page(runs: RunSummary[], nextCursor: string | null): TracesResponse {
  const template = response(10).traces[0]
  if (!template) throw new Error('Missing fixture result')
  return {
    traces: runs.map((run) => ({ ...template, run })),
    truncated: nextCursor !== null,
    nextCursor,
  }
}

function PagedSearch({
  runs,
  rev,
  id = '4bf92',
}: {
  runs: RunSummary[]
  rev: number
  id?: string
}) {
  const search = useTraceSearch(runs, rev, id, {
    method: 'locator.click',
    minMs: 500.1,
    maxMs: 1000,
  })
  return (
    <>
      <output>
        {search.kind === 'ready'
          ? search.answer.traces.map((row) => row.run.id).join(',')
          : search.kind}
      </output>
      <button
        type="button"
        onClick={search.loadMore}
        disabled={
          search.kind !== 'ready' ||
          search.loadingMore ||
          !search.answer.nextCursor
        }
      >
        More
      </button>
      {search.kind === 'ready' && search.failed && <p>Refresh failed</p>}
    </>
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
    await dom.interact(() =>
      held[3]?.({ traces: [], truncated: false, nextCursor: null }),
    )
    expect(dom.container.textContent).toBe('')
  })

  it('continues with every filter, refreshes all loaded rows through their last run, and resets on query changes', async () => {
    dom = await installDom()
    const runs = [runSummary(831), runSummary(832), runSummary(833)]
    let newest = runs[2]
    const paths: string[] = []
    globalThis.fetch = Object.assign(async (input: string | URL) => {
      const path = String(input)
      paths.push(path)
      const params = new URL(path, 'http://localhost').searchParams
      if (params.get('traceId') === 'dead') return Response.json(page([], null))
      return Response.json(
        params.has('cursor')
          ? page(
              newest === runs[2]
                ? runs.slice(0, 1)
                : runs.slice(0, 2).reverse(),
              null,
            )
          : page(
              newest
                ? [newest, newest === runs[2] ? runs[1] : runs[2]].filter(
                    (run) => run !== undefined,
                  )
                : [],
              'older',
            ),
      )
    }, globalThis.fetch)
    await dom.render(<PagedSearch runs={runs} rev={1} />)
    const output = dom.container.querySelector('output')
    const more = dom.container.querySelector('button')
    if (!more) throw new Error('Missing continuation button')
    expect(output?.textContent).toBe('run-833,run-832')
    await dom.interact(() => fire(more, 'click'))
    expect(output?.textContent).toBe('run-833,run-832,run-831')
    expect(paths.at(-1)).toBe(
      '/api/traces?traceId=4bf92&method=locator.click&minMs=500.1&maxMs=1000&cursor=older',
    )
    newest = runSummary(834)
    await dom.render(<PagedSearch runs={[...runs, newest]} rev={2} />)
    expect(output?.textContent).toBe('run-834,run-833,run-832,run-831')
    expect(more.hasAttribute('disabled')).toBe(true)
    await dom.render(<PagedSearch runs={runs} rev={2} id="dead" />)
    expect(output?.textContent).toBe('')
    expect(paths.at(-1)).toBe(
      '/api/traces?traceId=dead&method=locator.click&minMs=500.1&maxMs=1000',
    )
  })

  it('keeps the current page after a failed continuation and lets the button retry it', async () => {
    dom = await installDom()
    const runs = [runSummary(841), runSummary(842)]
    let fail = true
    globalThis.fetch = Object.assign(async (input: string | URL) => {
      const continuation = String(input).includes('cursor=')
      if (continuation && fail) return new Response('', { status: 500 })
      return Response.json(
        continuation
          ? page(runs.slice(0, 1), null)
          : page(runs.slice(1), 'older'),
      )
    }, globalThis.fetch)
    await dom.render(<PagedSearch runs={runs} rev={1} />)
    const more = dom.container.querySelector('button')
    if (!more) throw new Error('Missing continuation button')
    await dom.interact(() => fire(more, 'click'))
    expect(dom.container.querySelector('output')?.textContent).toBe('run-842')
    expect(dom.container.textContent).toContain('Refresh failed')
    expect(more.hasAttribute('disabled')).toBe(false)
    fail = false
    await dom.interact(() => fire(more, 'click'))
    expect(dom.container.querySelector('output')?.textContent).toBe(
      'run-842,run-841',
    )
    expect(dom.container.textContent).not.toContain('Refresh failed')
  })
})
