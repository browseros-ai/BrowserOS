import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  apiRoutes,
  type RunResponse,
  type TaskRunsResponse,
  type TasksResponse,
  type ViewerEvent,
  type ViewerInfo,
} from '../../shared/api'
import { runResult, sseEvent, startFakeNeo } from '../test-support/fake-neo'
import { startViewer } from '../viewer-server'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

test('the API lists tasks, a task with its runs, one run in full, and streams each change', async () => {
  const neo = startFakeNeo(() => ({
    chunks: [sseEvent(runResult(1, '"done"', { value: 'done' }))],
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
  const { tasks } = await get<TasksResponse>(apiRoutes.tasks)
  expect(tasks).toHaveLength(1)
  const taskId = tasks[0]?.id ?? ''
  const { task, runs } = await get<TaskRunsResponse>(apiRoutes.taskRuns(taskId))
  expect(task.totals).toMatchObject({ runs: 1, running: 0, failed: 0 })
  expect(runs.map((run) => run.status)).toEqual(['ok'])
  const { run } = await get<RunResponse>(apiRoutes.run(runs[0]?.id ?? ''))
  expect(run.reply.text).toBe('"done"')
  expect(
    events.map((event) =>
      event.type === 'run' ? `run:${event.run.status}` : 'task',
    ),
  ).toContain('run:running')
  expect(
    events.map((event) =>
      event.type === 'run' ? `run:${event.run.status}` : 'task',
    ),
  ).toContain('run:ok')

  expect(
    (await fetch(`${viewer.url}${apiRoutes.taskRuns('nope')}`)).status,
  ).toBe(404)
  // The page: built at start, its stylesheet compiled by Tailwind from the classes web/ uses.
  const html = await (await fetch(viewer.url)).text()
  const css = /href="\.\/([^"]+\.css)"/.exec(html)?.[1]
  expect(html).toContain('<div id="root"></div>')
  expect(await (await fetch(`${viewer.url}/${css}`)).text()).toContain(
    '.max-w-6xl',
  )
  // A page that rebinds its own domain to 127.0.0.1 cannot read the log.
  const foreign = await fetch(`${viewer.url}${apiRoutes.tasks}`, {
    headers: { host: 'attacker.example' },
  })
  expect(foreign.status).toBe(403)
})
