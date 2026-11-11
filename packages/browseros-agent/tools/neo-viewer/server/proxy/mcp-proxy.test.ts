import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { RunDetail, TaskSummary } from '../../shared/api'
import type { ProcessFinder } from '../tasks/peer-process'
import {
  type FakeNeo,
  type FakeReply,
  runResult,
  sseEvent,
  startFakeNeo,
} from '../test-support/fake-neo'
import { type RunningViewer, startViewer } from '../viewer-server'

const MCP_HEADERS = {
  'content-type': 'application/json',
  accept: 'application/json, text/event-stream',
  'mcp-protocol-version': '2026-07-28',
}

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

async function setup(
  respond: (body: string) => FakeReply,
  options: {
    processes?: (port: number) => ProcessFinder
    dataDir?: string
  } = {},
) {
  const neo = startFakeNeo((request) => respond(request.body))
  cleanups.push(() => neo.stop())
  const dataDir =
    options.dataDir ?? (await mkdtemp(join(tmpdir(), 'neo-viewer-test-')))
  if (!options.dataDir)
    cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer(
    { port: 0, upstream: new URL(neo.url), dataDir },
    options.processes ? { processes: options.processes } : {},
  )
  cleanups.push(() => viewer.stop())
  return { neo, viewer, dataDir }
}

async function post(
  url: string,
  body: string,
  headers: Record<string, string> = MCP_HEADERS,
) {
  const response = await fetch(url, { method: 'POST', headers, body })
  return {
    status: response.status,
    contentType: response.headers.get('content-type'),
    body: new Uint8Array(await response.arrayBuffer()),
  }
}

function runCall(id: number, code: string, meta?: unknown): string {
  return JSON.stringify({
    jsonrpc: '2.0',
    id,
    method: 'tools/call',
    params: {
      name: 'run',
      arguments: { code },
      _meta: {
        'io.modelcontextprotocol/clientInfo': {
          name: 'claude-code',
          version: '2.1.322',
        },
        ...(meta ?? {}),
      },
    },
  })
}

async function waitFor<T>(
  read: () => T | null | undefined | false,
  timeoutMs = 3000,
): Promise<T> {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const value = read()
    if (value) return value
    if (Date.now() > deadline) throw new Error('timed out waiting')
    await Bun.sleep(10)
  }
}

async function onlyRun(viewer: RunningViewer): Promise<RunDetail> {
  const task = await waitFor(() => viewer.store.listTasks()[0])
  return waitFor(() => {
    const runs = viewer.store.task(task.id)?.runs ?? []
    const done = runs.find((run) => run.status !== 'running')
    return done ? viewer.store.run(done.id) : null
  })
}

/** Sends the same request to Neo directly and through the viewer; both replies must be equal bytes. */
async function sameBytes(neo: FakeNeo, viewer: RunningViewer, body: string) {
  const direct = await post(neo.url, body)
  const proxied = await post(viewer.mcpUrl, body)
  expect(proxied.status).toBe(direct.status)
  expect(proxied.contentType).toBe(direct.contentType)
  expect(Buffer.from(proxied.body).equals(Buffer.from(direct.body))).toBe(true)
  return proxied
}

describe('a reply through the viewer is the direct reply, byte for byte', () => {
  test('a plain value, and the run is logged as sent and as received', async () => {
    const reply = sseEvent(runResult(1, '42', { value: 42 }))
    const { neo, viewer } = await setup(() => ({ chunks: [reply] }))
    const body = runCall(1, 'return 42')
    await sameBytes(neo, viewer, body)
    const run = await onlyRun(viewer)
    expect(run).toMatchObject({
      status: 'ok',
      code: 'return 42',
      taskIdStamped: true,
      codeSize: { chars: 9, tokens: 2 },
      replySize: { chars: 2, tokens: 0 },
      neoRunId: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
    })
    expect(run.reply.raw).toBe(reply)
    expect(run.reply.structuredContent).toEqual({ value: 42 })
    expect(run.request.raw).toBe(body)
    // Neo got the agent's bytes with only the task id added.
    const task = viewer.store.listTasks()[0] as TaskSummary
    const upstreamBody = neo.requests.at(-1)?.body
    expect(upstreamBody).toBe(
      body.replace(
        '"_meta":{',
        `"_meta":{"com.browseros.neo/run":{"taskId":${JSON.stringify(task.neoTaskId)}},`,
      ),
    )
    expect(run.request.forwarded).toBe(upstreamBody ?? null)
  })

  test('an error result', async () => {
    const { neo, viewer } = await setup(() => ({
      chunks: [
        sseEvent(
          runResult(
            2,
            'script / script_error: TypeError: x is not a function (run:1:1)',
            { error: {} },
            true,
          ),
        ),
      ],
    }))
    await sameBytes(neo, viewer, runCall(2, 'x()'))
    expect((await onlyRun(viewer)).status).toBe('error')
  })

  test('a reply with console lines', async () => {
    const text = '7\nconsole:\nhi\n[warn] careful'
    const { neo, viewer } = await setup(() => ({
      chunks: [sseEvent(runResult(3, text, { value: 7 }))],
    }))
    await sameBytes(
      neo,
      viewer,
      runCall(3, 'console.log("hi"); console.warn("careful"); return 7'),
    )
    expect((await onlyRun(viewer)).reply.text).toBe(text)
  })

  test('a large reply with a blob marker, in many chunks', async () => {
    const marker = { $neo: 'blob', id: 'b0', size: 300000 }
    const big = sseEvent(
      runResult(4, `${JSON.stringify(marker)}${'x'.repeat(400_000)}`, {
        value: marker,
      }),
    )
    const pieces = big.match(/[\s\S]{1,16384}/g) ?? []
    const { neo, viewer } = await setup(() => ({ chunks: pieces }))
    await sameBytes(
      neo,
      viewer,
      runCall(4, 'return Buffer.from("x".repeat(300000))'),
    )
    expect((await onlyRun(viewer)).reply.raw).toBe(big)
  })

  test('a streamed reply reaches the agent as it streams: keep-alives and notifications first', async () => {
    const progress = {
      jsonrpc: '2.0',
      method: 'notifications/progress',
      params: { progressToken: 1, progress: 1 },
    }
    const { neo, viewer } = await setup(() => ({
      chunks: [
        ':\n\n',
        250,
        sseEvent(progress),
        250,
        ':\n\n',
        250,
        sseEvent(runResult(5, '1', { value: 1 })),
      ],
    }))
    const body = runCall(5, 'await page.waitForTimeout(6000); return 1')
    await sameBytes(neo, viewer, body)
    // Through the viewer, the first chunk arrives long before Neo sends the last one.
    const response = await fetch(viewer.mcpUrl, {
      method: 'POST',
      headers: MCP_HEADERS,
      body,
    })
    const reader = response.body?.getReader()
    const first = await reader?.read()
    const firstAt = Date.now()
    while (!(await reader?.read())?.done) {}
    const lastSentAt = neo.requests.at(-1)?.sentAt.at(-1) ?? 0
    expect(new TextDecoder().decode(first?.value)).toBe(':\n\n')
    expect(lastSentAt - firstAt).toBeGreaterThan(500)
    const runs = await waitFor(() => {
      const task = viewer.store.listTasks()[0]
      const list = task ? (viewer.store.task(task.id)?.runs ?? []) : []
      return list.length === 2 && list.every((run) => run.status === 'ok')
        ? list
        : null
    })
    const detail = viewer.store.run(runs[1]?.id ?? '')
    expect(detail?.reply.keepAlives).toBe(2)
    expect(detail?.reply.notifications).toEqual([progress])
  })

  test('a JSON-RPC error, a 202 for a notification and a refusal pass through as sent', async () => {
    const { neo, viewer } = await setup((body): FakeReply => {
      if (body.includes('notifications/initialized'))
        return { status: 202, headers: { 'content-length': '0' }, chunks: [] }
      if (body.includes('"refuse"')) {
        return {
          status: 406,
          headers: { 'content-type': 'text/plain', 'content-length': '78' },
          chunks: [
            'Not Acceptable: Client must accept both application/json and text/event-stream',
          ],
        }
      }
      return {
        chunks: [
          sseEvent({
            jsonrpc: '2.0',
            id: 6,
            error: { code: -32602, message: 'unknown tool' },
          }),
        ],
      }
    })
    await sameBytes(
      neo,
      viewer,
      '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    )
    await sameBytes(neo, viewer, '{"jsonrpc":"2.0","id":9,"method":"refuse"}')
    await sameBytes(neo, viewer, runCall(6, 'return 1'))
    const run = await onlyRun(viewer)
    expect(run.status).toBe('rpc-error')
    expect(run.reply.rpcError).toEqual({
      code: -32602,
      message: 'unknown tool',
    })
  })
})

test('a compressed reply passes through compressed, as Neo sent it', async () => {
  const gzipped = Bun.gzipSync(sseEvent(runResult(7, '7', { value: 7 })))
  const { viewer } = await setup(() => ({
    headers: {
      'content-type': 'text/event-stream',
      'content-encoding': 'gzip',
    },
    chunks: [gzipped],
  }))
  const response = await fetch(viewer.mcpUrl, {
    method: 'POST',
    headers: MCP_HEADERS,
    body: runCall(7, 'return 7'),
    decompress: false,
  } as RequestInit)
  expect(response.headers.get('content-encoding')).toBe('gzip')
  expect(
    Buffer.from(await response.arrayBuffer()).equals(Buffer.from(gzipped)),
  ).toBe(true)
  // The log reads its own decoded copy.
  const run = await onlyRun(viewer)
  expect(run).toMatchObject({
    status: 'ok',
    neoRunId: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
  })
  expect(run.reply).toMatchObject({
    text: '7',
    structuredContent: { value: 7 },
    contentEncoding: 'gzip',
  })
  expect(run.reply.raw).toBe(sseEvent(runResult(7, '7', { value: 7 })))
})

test('a sized reply still streams: the first bytes reach the agent before the rest is sent', async () => {
  // Bun's server always frames a streamed body as chunked, so this Neo writes HTTP by hand.
  const first = ':\n\n'
  const rest = sseEvent(runResult(8, '8', { value: 8 }))
  let restSentAt = 0
  const neo = Bun.listen({
    hostname: '127.0.0.1',
    port: 0,
    socket: {
      data(socket) {
        socket.write(
          `HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: ${first.length + rest.length}\r\n\r\n${first}`,
        )
        setTimeout(() => {
          restSentAt = Date.now()
          socket.write(rest)
          socket.end()
        }, 600)
      },
    },
  })
  cleanups.push(async () => neo.stop(true))
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-test-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer({
    port: 0,
    upstream: new URL(`http://127.0.0.1:${neo.port}/mcp`),
    dataDir,
  })
  cleanups.push(() => viewer.stop())
  const response = await fetch(viewer.mcpUrl, {
    method: 'POST',
    headers: MCP_HEADERS,
    body: runCall(8, 'return 8'),
  })
  const reader = response.body?.getReader()
  const head = await reader?.read()
  const headAt = Date.now()
  let body = new TextDecoder().decode(head?.value)
  for (;;) {
    const chunk = await reader?.read()
    if (!chunk || chunk.done) break
    body += new TextDecoder().decode(chunk.value)
  }
  expect(body).toBe(first + rest)
  expect(restSentAt - headAt).toBeGreaterThan(300)
  expect((await onlyRun(viewer)).reply.keepAlives).toBe(1)
})

describe('tasks', () => {
  test("a call that names its own task is forwarded untouched and joins the agent's task", async () => {
    const { neo, viewer } = await setup(() => ({
      chunks: [sseEvent(runResult(1, '1', { value: 1 }))],
    }))
    const body = runCall(1, 'return 1', {
      'com.browseros.neo/run': { taskId: 'zuzu-42', requestId: 'rq-1' },
    })
    await post(viewer.mcpUrl, body)
    expect(neo.requests.at(-1)?.body).toBe(body)
    const run = await onlyRun(viewer)
    const task = viewer.store.listTasks()[0]
    expect(run.taskIdStamped).toBe(false)
    expect(task?.identifiedBy).toEqual({
      kind: 'agent-task-id',
      taskId: 'zuzu-42',
    })
    expect(task?.neoTaskId).toBe('zuzu-42')
    expect(task?.name).toMatch(/^claude-code, \d\d:\d\d$/)
  })

  test('without a process, calls group by client name and say why', async () => {
    const processes = () => ({
      find: () => ({
        lookup: Promise.resolve({ found: false as const, reason: 'test' }),
        release: () => {},
      }),
    })
    const { viewer } = await setup(
      () => ({ chunks: [sseEvent(runResult(1, '1', { value: 1 }))] }),
      { processes },
    )
    await post(viewer.mcpUrl, runCall(1, 'return 1'))
    await post(viewer.mcpUrl, runCall(1, 'return 2'))
    const task = await waitFor(() =>
      viewer.store.listTasks().find((candidate) => candidate.totals.runs === 2),
    )
    expect(task.identifiedBy).toEqual({
      kind: 'client-idle',
      idleMinutes: 10,
      reason: 'test',
    })
    expect(viewer.store.listTasks()).toHaveLength(1)
  })

  test('the gap before a run is the time since the previous reply; tools/list size is kept on the task', async () => {
    const list = {
      jsonrpc: '2.0',
      id: 0,
      result: {
        tools: [
          {
            name: 'run',
            description: 'd'.repeat(400),
            inputSchema: { type: 'object' },
          },
        ],
      },
    }
    const { viewer } = await setup((body) =>
      body.includes('tools/list')
        ? { chunks: [sseEvent(list)] }
        : { chunks: [sseEvent(runResult(1, '1', { value: 1 }))] },
    )
    await post(
      viewer.mcpUrl,
      JSON.stringify({
        jsonrpc: '2.0',
        id: 0,
        method: 'tools/list',
        params: {
          _meta: {
            'io.modelcontextprotocol/clientInfo': { name: 'claude-code' },
          },
        },
      }),
    )
    await post(viewer.mcpUrl, runCall(1, 'return 1'))
    await waitFor(() => viewer.store.listTasks()[0]?.totals.running === 0)
    await Bun.sleep(120)
    await post(viewer.mcpUrl, runCall(2, 'return 2'))
    const task = await waitFor(() => {
      const found = viewer.store.listTasks()[0]
      return found?.totals.runs === 2 && found.totals.running === 0
        ? found
        : null
    })
    const runs = viewer.store.task(task.id)?.runs ?? []
    expect(runs[0]?.gapMs).toBeNull()
    expect(runs[1]?.gapMs).toBeGreaterThanOrEqual(100)
    expect(task.toolsList).toEqual({
      chars: 'run'.length + 400 + '{"type":"object"}'.length,
      tokens: 105,
    })
  })

  test('two agent processes at once become two tasks and never mix', async () => {
    const { viewer } = await setup((body) => {
      const id = (JSON.parse(body) as { id: number }).id
      return {
        chunks: [40, sseEvent(runResult(id, String(id), { value: id }))],
      }
    })
    const agent = (name: string) =>
      Bun.spawn(
        [
          process.execPath,
          '-e',
          `for (let i = 1; i <= 4; i++) {
             await fetch(${JSON.stringify(viewer.mcpUrl)}, { method: 'POST', headers: ${JSON.stringify(MCP_HEADERS)},
               body: JSON.stringify({ jsonrpc: '2.0', id: i, method: 'tools/call', params: { name: 'run',
                 arguments: { code: '// ${name} ' + i }, _meta: { 'io.modelcontextprotocol/clientInfo': { name: 'claude-code' } } } }) })
               .then((r) => r.text())
           }`,
        ],
        { stdout: 'ignore', stderr: 'inherit' },
      )
    const children = [agent('A'), agent('B')]
    cleanups.push(async () => {
      for (const child of children) child.kill()
    })
    expect(await Promise.all(children.map((child) => child.exited))).toEqual([
      0, 0,
    ])
    const tasks = await waitFor(() => {
      const list = viewer.store.listTasks()
      return list.length === 2 &&
        list.every(
          (task) => task.totals.runs === 4 && task.totals.running === 0,
        )
        ? list
        : null
    })
    const pids = tasks.map((task) =>
      task.identifiedBy.kind === 'process' ? task.identifiedBy.pid : null,
    )
    expect(new Set(pids)).toEqual(new Set(children.map((child) => child.pid)))
    for (const task of tasks) {
      const agents = new Set(
        viewer.store.task(task.id)?.runs.map((run) => run.code.split(' ')[1]),
      )
      expect(agents.size).toBe(1)
    }
  })

  test('a restart reads the log back: the same tasks and runs, and the next call joins its task', async () => {
    const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-test-'))
    cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
    const respond = () => ({
      chunks: [sseEvent(runResult(1, '1', { value: 1 }))],
    })
    const first = await setup(respond, { dataDir })
    await post(first.viewer.mcpUrl, runCall(1, 'return 1'))
    const before = (await onlyRun(first.viewer)).taskId
    await first.viewer.stop()
    const second = await setup(respond, { dataDir })
    expect(second.viewer.store.listTasks().map((task) => task.id)).toEqual([
      before,
    ])
    expect(second.viewer.store.task(before)?.runs[0]?.status).toBe('ok')
    await post(second.viewer.mcpUrl, runCall(2, 'return 2'))
    await waitFor(() => second.viewer.store.task(before)?.runs.length === 2)
  })
})

describe('the agent going away', () => {
  test('closes the upstream request, so Neo stops the run', async () => {
    const { neo, viewer } = await setup(() => ({
      chunks: [':\n\n', 5000, sseEvent(runResult(1, '1', { value: 1 }))],
    }))
    const abort = new AbortController()
    const response = await fetch(viewer.mcpUrl, {
      method: 'POST',
      headers: MCP_HEADERS,
      body: runCall(1, 'await page.waitForTimeout(60000)'),
      signal: abort.signal,
    })
    await response.body?.getReader().read()
    abort.abort()
    await waitFor(() => neo.requests[0]?.aborted)
    const run = await onlyRun(viewer)
    expect(run.status).toBe('cancelled')
  })

  test('Neo not answering is a 502 from the viewer and an http-error run', async () => {
    const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-test-'))
    cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
    const viewer = await startViewer({
      port: 0,
      upstream: new URL('http://127.0.0.1:1/mcp'),
      dataDir,
    })
    cleanups.push(() => viewer.stop())
    const reply = await post(viewer.mcpUrl, runCall(1, 'return 1'))
    expect(reply.status).toBe(502)
    expect((await onlyRun(viewer)).status).toBe('http-error')
  })
})
