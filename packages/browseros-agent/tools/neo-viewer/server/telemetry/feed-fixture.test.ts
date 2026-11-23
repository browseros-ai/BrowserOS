import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  apiRoutes,
  type MetricsResponse,
  type RunResponse,
  type RunTraceResponse,
} from '../../shared/api'
import { startViewer } from '../viewer-server'
import { feed, serveStandIn } from './feed-fixture'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

async function viewerOn(withHostApi: boolean) {
  const standIn = await serveStandIn(0)
  cleanups.push(() => standIn.stop())
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-feed-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer(
    {
      port: 0,
      upstream: new URL(`${standIn.url}/mcp`),
      dataDir,
      hostApi: withHostApi
        ? { url: new URL(standIn.url), tokenFile: standIn.tokenFile }
        : null,
    },
    { telemetry: { scrape: { intervalMs: 20 } } },
  )
  cleanups.push(() => viewer.stop())
  const get = async <T>(path: string) =>
    (await (await fetch(`${viewer.url}${path}`)).json()) as T
  return { viewer, get }
}

test('the feeder against the stand-in, spans mode: one run, its eight ops from spans, its trace, pushed metrics', async () => {
  const { viewer, get } = await viewerOn(false)
  const fed = await feed({ viewer: viewer.url, gzip: true })
  expect(fed.traces).toBe(200)
  expect(fed.run.trace?.injected).toBe(true)
  const { run } = await get<RunResponse>(
    apiRoutes.run(fed.run.id, { waitForNeo: true }),
  )
  expect(run.neo).toMatchObject({ source: 'spans', complete: true })
  expect(run.operationCount).toBe(8)
  expect(
    run.neo?.operations.filter((op) => op.outcome === 'error'),
  ).toHaveLength(1)
  const { trace } = await get<RunTraceResponse>(apiRoutes.runTrace(fed.run.id))
  expect(trace?.spans).toHaveLength(9)
  const { metrics } = await get<MetricsResponse>(apiRoutes.metrics)
  expect(metrics?.source).toBe('push')
  expect(metrics?.families).toHaveLength(16)
})

test('the feeder against the stand-in, host mode: the op table is the host record, the health the scrape', async () => {
  const { viewer, get } = await viewerOn(true)
  const fed = await feed({ viewer: viewer.url })
  const { run } = await get<RunResponse>(
    apiRoutes.run(fed.run.id, { waitForNeo: true }),
  )
  expect(run.neo?.source).toBeUndefined()
  expect(run.neo?.operations).toHaveLength(8)
  expect(run.neo?.operations[0]?.args).toEqual({ url: 'https://example.com' })
  const { trace } = await get<RunTraceResponse>(apiRoutes.runTrace(fed.run.id))
  expect(trace?.complete).toBe(true)
  for (let i = 0; i < 100; i++) {
    const { metrics } = await get<MetricsResponse>(apiRoutes.metrics)
    if (metrics?.source === 'scrape') return
    await Bun.sleep(20)
  }
  throw new Error('the scrape never became the source')
})
