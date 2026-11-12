import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  apiRoutes,
  type RunResponse,
  type RunsResponse,
  type ViewerEvent,
  type ViewerInfo,
} from '../../shared/api'
import { runMarkdown } from '../../shared/run-markdown'
import { runResult, sseEvent, startFakeNeo } from '../test-support/fake-neo'
import { FAKE_TOKEN } from '../test-support/fake-neo-host'
import { startViewer } from '../viewer-server'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

test('the API lists tasks, a task with its runs, one run in full, and streams each change', async () => {
  const neo = startFakeNeo(() => ({
    chunks: [sseEvent(runResult(1, '### Result\n"done"'))],
  }))
  cleanups.push(() => neo.stop())
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-api-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer({
    port: 0,
    upstream: new URL(neo.url),
    dataDir,
  })
  cleanups.push(() => viewer.stop())
  const get = async <T>(path: string) =>
    (await (await fetch(`${viewer.url}${path}`)).json()) as T

  const events: ViewerEvent[] = []
  const stream = await fetch(`${viewer.url}${apiRoutes.events}`)
  const reader = stream.body?.getReader()
  const reading = (async () => {
    let buffer = ''
    for (;;) {
      const chunk = await reader?.read()
      if (!chunk || chunk.done) return
      buffer += new TextDecoder().decode(chunk.value)
      for (const block of buffer.split('\n\n').slice(0, -1)) {
        if (block.startsWith('data: '))
          events.push(JSON.parse(block.slice(6)) as ViewerEvent)
      }
      buffer = buffer.slice(buffer.lastIndexOf('\n\n') + 2)
    }
  })()
  cleanups.push(async () => {
    await reader?.cancel()
    await reading
  })

  await fetch(viewer.mcpUrl, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: { name: 'run', arguments: { code: 'return "done"' } },
    }),
  }).then((response) => response.text())

  const info = await get<ViewerInfo>(apiRoutes.info)
  expect(info.mcpUrl).toBe(viewer.mcpUrl)
  expect(info.upstreamUrl).toBe(neo.url)

  for (
    let i = 0;
    i < 100 &&
    !events.some((event) => event.type === 'run' && event.run.status === 'ok');
    i++
  ) {
    await Bun.sleep(10)
  }
  const { runs, log, version } = await get<RunsResponse>(apiRoutes.runs)
  // The snapshot includes every change announced so far.
  expect(version).toBeGreaterThanOrEqual(
    Math.max(...events.map((event) => event.version)),
  )
  expect(log.totals).toMatchObject({ runs: 1, running: 0, failed: 0 })
  expect(runs.map((run) => run.status)).toEqual(['ok'])
  const { run } = await get<RunResponse>(apiRoutes.run(runs[0]?.id ?? ''))
  expect(run.reply.text).toBe('### Result\n"done"')
  const kinds = events.map((event) =>
    event.type === 'run' ? `run:${event.run.status}` : event.type,
  )
  expect(kinds).toContain('run:running')
  expect(kinds).toContain('run:ok')
  expect((await fetch(`${viewer.url}${apiRoutes.run('nope')}`)).status).toBe(
    404,
  )

  // Clear: only from the viewer's own page; it empties the list and tells every open page.
  const foreignClear = await fetch(`${viewer.url}${apiRoutes.clear}`, {
    method: 'POST',
    headers: { origin: 'https://attacker.example' },
  })
  expect(foreignClear.status).toBe(403)
  const cleared = await fetch(`${viewer.url}${apiRoutes.clear}`, {
    method: 'POST',
    headers: { origin: viewer.url },
  })
  expect(((await cleared.json()) as RunsResponse).runs).toEqual([])
  for (
    let i = 0;
    i < 100 && !events.some((event) => event.type === 'clear');
    i++
  ) {
    await Bun.sleep(10)
  }
  expect(events.at(-1)).toMatchObject({
    type: 'clear',
    log: { totals: { runs: 0 } },
  })
  expect((await get<RunsResponse>(apiRoutes.runs)).runs).toEqual([])
  // The log before the clear is kept, once, for a mis-click.
  expect(await Bun.file(join(dataDir, 'log.prev.jsonl')).text()).toContain(
    'return \\"done\\"',
  )

  // The page: built at start, its stylesheet compiled by Tailwind from the classes web/ uses.
  const html = await (await fetch(viewer.url)).text()
  const css = /href="\.\/([^"]+\.css)"/.exec(html)?.[1]
  expect(html).toContain('<div id="root"></div>')
  expect(await (await fetch(`${viewer.url}/${css}`)).text()).toContain(
    '.max-w-6xl',
  )
  // A page that rebinds its own domain to 127.0.0.1 cannot read the log.
  const foreign = await fetch(`${viewer.url}${apiRoutes.runs}`, {
    headers: { host: 'attacker.example' },
  })
  expect(foreign.status).toBe(403)
})

/** A viewer whose Neo answers a run at once and whose host API gives its record after `recordDelayMs`. */
async function viewerWithSlowRecord(recordDelayMs: number, waitMs: number) {
  const neo = startFakeNeo(() => ({
    chunks: [sseEvent(runResult(1, '### Result\n1'))],
  }))
  cleanups.push(() => neo.stop())
  const record = {
    id: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
    status: 'ok',
    startedAt: 1_000,
    endedAt: 1_300,
    complete: true,
    tabs: [7],
    operations: [
      {
        id: 'x.1',
        name: 'page.goto',
        kind: 'builtin',
        effect: 'write',
        site: { line: 1, column: 7 },
        startedAt: 1_010,
        durationMs: 250,
        outcome: 'ok',
      },
    ],
  }
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-api-wait-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const tokenFile = join(dataDir, 'host-token')
  await writeFile(tokenFile, FAKE_TOKEN, { mode: 0o600 })
  // Neo's host API, answering `GET /v1/runs/{id}` (the only call the joiner makes) after the delay.
  const slow = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(req) {
      if (req.headers.get('authorization') !== `Bearer ${FAKE_TOKEN}`)
        return new Response('unauthorized', { status: 401 })
      if (!new URL(req.url).pathname.startsWith('/v1/runs/'))
        return new Response('not found', { status: 404 })
      await Bun.sleep(recordDelayMs)
      return Response.json(record)
    },
  })
  cleanups.push(async () => slow.stop(true))
  const viewer = await startViewer(
    {
      port: 0,
      upstream: new URL(neo.url),
      dataDir,
      hostApi: { url: new URL(`http://127.0.0.1:${slow.port}`), tokenFile },
    },
    { neoRecordWaitMs: waitMs },
  )
  cleanups.push(() => viewer.stop())
  await fetch(viewer.mcpUrl, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: {
        name: 'run',
        arguments: { code: "await page.goto('https://example.com')" },
      },
    }),
  }).then((response) => response.text())
  const runId = viewer.store.list()[0]?.id ?? ''
  return { viewer, runId }
}

test('?wait=neo: a run that just ended is answered once Neo’s record has joined', async () => {
  const { viewer, runId } = await viewerWithSlowRecord(400, 3_000)
  // Without the wait the record is not there yet.
  const plain = (await (
    await fetch(`${viewer.url}${apiRoutes.run(runId)}`)
  ).json()) as RunResponse
  expect(plain.run.neo).toBeNull()
  const started = Date.now()
  const waited = (await (
    await fetch(`${viewer.url}${apiRoutes.run(runId, { waitForNeo: true })}`)
  ).json()) as RunResponse
  expect(Date.now() - started).toBeLessThan(2_000)
  expect(waited.run.operationCount).toBe(1)
  expect(runMarkdown(waited.run, { replyChars: null })).toContain(
    '- L1 page.goto: +10 ms, 250 ms, ok',
  )
})

test('?wait=neo is bounded: a record that does not come in time is copied as not arrived', async () => {
  const { viewer, runId } = await viewerWithSlowRecord(2_000, 300)
  const started = Date.now()
  const waited = (await (
    await fetch(`${viewer.url}${apiRoutes.run(runId, { waitForNeo: true })}`)
  ).json()) as RunResponse
  const took = Date.now() - started
  expect(took).toBeGreaterThanOrEqual(250)
  expect(took).toBeLessThan(1_500)
  expect(waited.run.neo).toBeNull()
  expect(
    runMarkdown(waited.run, { replyChars: null }).split('\n')[0],
  ).toContain("Neo's record had not arrived")
})
