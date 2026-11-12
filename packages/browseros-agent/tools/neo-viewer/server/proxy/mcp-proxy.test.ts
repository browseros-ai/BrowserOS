import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { RunDetail } from '../../shared/api'
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
  options: { dataDir?: string } = {},
) {
  const neo = startFakeNeo((request) => respond(request.body))
  cleanups.push(() => neo.stop())
  const dataDir =
    options.dataDir ?? (await mkdtemp(join(tmpdir(), 'neo-viewer-test-')))
  if (!options.dataDir)
    cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer({
    port: 0,
    upstream: new URL(neo.url),
    dataDir,
  })
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
  return waitFor(() => {
    const done = viewer.store.list().find((run) => run.status !== 'running')
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
    const reply = sseEvent(runResult(1, '### Result\n42'))
    const { neo, viewer } = await setup(() => ({ chunks: [reply] }))
    const body = runCall(1, 'return 42')
    await sameBytes(neo, viewer, body)
    const run = await onlyRun(viewer)
    expect(run).toMatchObject({
      status: 'ok',
      code: 'return 42',
      client: { name: 'claude-code', version: '2.1.322' },
      agentTaskId: null,
      codeSize: { chars: 9, tokens: 2 },
      replySize: { chars: 13, tokens: 3 },
      neoRunId: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
    })
    expect(run.reply.raw).toBe(reply)
    expect(run.reply.structuredContent).toBeNull()
    expect(JSON.parse(reply.slice(6)).result).not.toHaveProperty(
      'structuredContent',
    )
    expect(run.request.raw).toBe(body)
    // Neo got the agent's bytes, every one: the viewer adds nothing.
    expect(neo.requests.at(-1)?.body).toBe(body)
  })

  test('an error result', async () => {
    const { neo, viewer } = await setup(() => ({
      chunks: [
        sseEvent(
          runResult(
            2,
            '### Error\nscript / script_error: TypeError: x is not a function (run:1:1)',
            true,
          ),
        ),
      ],
    }))
    await sameBytes(neo, viewer, runCall(2, 'x()'))
    expect((await onlyRun(viewer)).status).toBe('error')
  })

  test('a reply with console lines', async () => {
    const text = '### Result\n7\n### Console\nhi\n[warn] careful'
    const { neo, viewer } = await setup(() => ({
      chunks: [sseEvent(runResult(3, text))],
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
      runResult(
        4,
        `### Result\n${JSON.stringify({ marker, padding: 'x'.repeat(400_000) })}`,
      ),
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
        sseEvent(runResult(5, '### Result\n1')),
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
      const list = viewer.store.list()
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
  const gzipped = Bun.gzipSync(sseEvent(runResult(7, '### Result\n7')))
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
    text: '### Result\n7',
    structuredContent: null,
    contentEncoding: 'gzip',
  })
  expect(run.reply.raw).toBe(sseEvent(runResult(7, '### Result\n7')))
})

test('a sized reply still streams: the first bytes reach the agent before the rest is sent', async () => {
  // Bun's server always frames a streamed body as chunked, so this Neo writes HTTP by hand.
  const first = ':\n\n'
  const rest = sseEvent(runResult(8, '### Result\n8'))
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

describe('one log', () => {
  test('thirteen calls from thirteen processes are thirteen runs in one list, in order, each reaching Neo as sent', async () => {
    const { neo, viewer } = await setup((body) => {
      const id = (JSON.parse(body) as { id: number }).id
      return { chunks: [sseEvent(runResult(id, `### Result\n${id}`))] }
    })
    // Like an agent that calls the MCP URL from a new curl or Python process every time.
    const sent: string[] = []
    for (let i = 1; i <= 13; i++) {
      const body = JSON.stringify({
        jsonrpc: '2.0',
        id: i,
        method: 'tools/call',
        params: { name: 'run', arguments: { code: `return ${i}` } },
      })
      sent.push(body)
      const child = Bun.spawn(
        [
          'curl',
          '-sS',
          '-o',
          '/dev/null',
          '-X',
          'POST',
          viewer.mcpUrl,
          '-H',
          'content-type: application/json',
          '-H',
          'accept: application/json, text/event-stream',
          '-d',
          body,
        ],
        { stdout: 'ignore', stderr: 'inherit' },
      )
      expect(await child.exited).toBe(0)
    }
    const runs = await waitFor(() => {
      const list = viewer.store.list()
      return list.length === 13 && list.every((run) => run.status === 'ok')
        ? list
        : null
    })
    expect(runs.map((run) => [run.seq, run.code])).toEqual(
      sent.map((_, index) => [index + 1, `return ${index + 1}`]),
    )
    expect(
      runs.every(
        (run) => run.client?.name === 'curl' && run.agentTaskId === null,
      ),
    ).toBe(true)
    // No _meta added: Neo received each body exactly as curl sent it.
    expect(neo.requests.map((request) => request.body)).toEqual(sent)
    expect(viewer.store.summary().totals.runs).toBe(13)
  })

  test('a call that carries its own taskId passes through unchanged, and the run keeps it as a label', async () => {
    const { neo, viewer } = await setup(() => ({
      chunks: [sseEvent(runResult(1, '### Result\n1'))],
    }))
    const body = runCall(1, 'return 1', {
      'com.browseros.neo/run': { taskId: 'zuzu-42', requestId: 'rq-1' },
    })
    await post(viewer.mcpUrl, body)
    expect(neo.requests.at(-1)?.body).toBe(body)
    expect(await onlyRun(viewer)).toMatchObject({
      agentTaskId: 'zuzu-42',
      client: { name: 'claude-code', version: '2.1.322' },
    })
  })

  test('a legacy client is labelled from its initialize, an unnamed one from its User-Agent', async () => {
    const { viewer } = await setup(() => ({
      chunks: [sseEvent(runResult(1, '### Result\n1'))],
    }))
    const legacy = { ...MCP_HEADERS, 'user-agent': 'Python-urllib/3.12' }
    await post(
      viewer.mcpUrl,
      JSON.stringify({
        jsonrpc: '2.0',
        id: 0,
        method: 'initialize',
        params: { clientInfo: { name: 'zuzu-recipes', version: '1' } },
      }),
      legacy,
    )
    await post(
      viewer.mcpUrl,
      JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: { name: 'run', arguments: { code: 'return 1' } },
      }),
      legacy,
    )
    await post(
      viewer.mcpUrl,
      JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: { name: 'run', arguments: { code: 'return 2' } },
      }),
      { ...MCP_HEADERS, 'user-agent': 'node-fetch/1.0' },
    )
    const runs = await waitFor(() =>
      viewer.store.list().length === 2 ? viewer.store.list() : null,
    )
    expect(runs.map((run) => run.client?.name)).toEqual([
      'zuzu-recipes',
      'node-fetch',
    ])
  })

  test('the gap before a run is the time since the previous reply; the latest tools/list size is kept', async () => {
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
        : {
            chunks: [
              sseEvent(
                runResult(
                  (JSON.parse(body) as { id: number }).id,
                  '### Result\n1',
                ),
              ),
            ],
          },
    )
    await post(
      viewer.mcpUrl,
      JSON.stringify({
        jsonrpc: '2.0',
        id: 0,
        method: 'tools/list',
        params: {},
      }),
    )
    await post(viewer.mcpUrl, runCall(1, 'return 1'))
    await waitFor(
      () =>
        viewer.store.summary().totals.running === 0 &&
        viewer.store.list().length === 1,
    )
    await Bun.sleep(120)
    await post(viewer.mcpUrl, runCall(2, 'return 2'))
    const runs = await waitFor(() => {
      const found = viewer.store.list()
      return found.length === 2 && found.every((run) => run.status === 'ok')
        ? found
        : null
    })
    expect(runs[0]?.gapMs).toBeNull()
    expect(runs[1]?.gapMs).toBeGreaterThanOrEqual(100)
    expect(viewer.store.summary().toolsList).toEqual({
      chars: 'run'.length + 400 + '{"type":"object"}'.length,
      tokens: 105,
    })
  })

  test('a restart reads the log back, and the next call joins the same list', async () => {
    const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-test-'))
    cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
    const respond = () => ({
      chunks: [sseEvent(runResult(1, '### Result\n1'))],
    })
    const first = await setup(respond, { dataDir })
    await post(first.viewer.mcpUrl, runCall(1, 'return 1'))
    await onlyRun(first.viewer)
    await first.viewer.stop()
    const second = await setup(respond, { dataDir })
    expect(
      second.viewer.store.list().map((run) => [run.seq, run.status]),
    ).toEqual([[1, 'ok']])
    await post(second.viewer.mcpUrl, runCall(2, 'return 2'))
    await waitFor(() => second.viewer.store.list().length === 2)
    expect(second.viewer.store.list().map((run) => run.seq)).toEqual([1, 2])
  })
})

describe('the agent going away', () => {
  test('closes the upstream request, so Neo stops the run', async () => {
    const { neo, viewer } = await setup(() => ({
      chunks: [':\n\n', 5000, sseEvent(runResult(1, '### Result\n1'))],
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
