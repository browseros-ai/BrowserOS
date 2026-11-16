import { expect, spyOn, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { jsonObject, stringField } from './json'
import { createRunClient, parseRunReply } from './mcp'
import type { JsonValue } from './model'

function success(value: JsonValue) {
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
    structuredContent: { value },
    _meta: {
      'com.browseros.neo/run': {
        id: randomUUID(),
        status: 'ok',
        effects: 'done',
        unknown: [],
      },
    },
  }
}

test('run parser requires SDK metadata and consistent structured envelopes', () => {
  expect(parseRunReply(success({ kept: [7, true, null] }))).toMatchObject({
    kind: 'ok',
    value: { kept: [7, true, null] },
  })
  expect(() => parseRunReply({ content: [] })).toThrow('SDK run metadata')
  expect(() => parseRunReply({ ...success(true), isError: true })).toThrow(
    'status ok',
  )
  expect(() =>
    parseRunReply({ ...success(true), structuredContent: {} }),
  ).toThrow('omitted structured value')
})

test('script failures retain owner, code, status and full numeric source suffix', () => {
  const parsed = parseRunReply({
    content: [
      {
        type: 'text',
        text: 'script / script_error: Error: run-boom (run:2:5)',
      },
    ],
    structuredContent: {
      error: {
        owner: 'script',
        code: 'script_error',
        message: 'Error: run-boom (run:2:5)',
      },
    },
    isError: true,
    _meta: { 'com.browseros.neo/run': { id: randomUUID(), status: 'error' } },
  })
  expect(parsed).toMatchObject({
    kind: 'error',
    isError: true,
    status: 'error',
    error: {
      owner: 'script',
      code: 'script_error',
      message: 'Error: run-boom (run:2:5)',
    },
  })
})

for (const { name, facts } of [
  {
    name: 'unknown effects after a caught write error',
    facts: { effects: 'unknown', unknown: [] },
  },
  {
    name: 'unconfirmed writes even if effects says done',
    facts: {
      effects: 'done',
      unknown: [{ operationId: `${randomUUID()}.1`, name: 'locator.click' }],
    },
  },
]) {
  test(`run parser rejects ${name} despite status ok`, () => {
    const reply = success({ caught: true })
    expect(() =>
      parseRunReply({
        ...reply,
        _meta: {
          'com.browseros.neo/run': {
            ...reply._meta['com.browseros.neo/run'],
            ...facts,
          },
        },
      }),
    ).toThrow('uncertain')
  })
}

for (const unknown of [null, {}, 'pending', 1]) {
  test(`run parser rejects malformed supplied unknown list ${JSON.stringify(unknown)}`, () => {
    const reply = success(true)
    expect(() =>
      parseRunReply({
        ...reply,
        _meta: {
          'com.browseros.neo/run': {
            ...reply._meta['com.browseros.neo/run'],
            unknown,
          },
        },
      }),
    ).toThrow('unknown must be an array')
  })
}

test('run parser accepts absent or empty operation lists and completed effects', () => {
  for (const facts of [
    {},
    { effects: 'none' },
    { effects: 'done', unknown: [] },
  ]) {
    expect(
      parseRunReply({
        ...success(true),
        _meta: {
          'com.browseros.neo/run': { id: randomUUID(), status: 'ok', ...facts },
        },
      }).kind,
    ).toBe('ok')
  }
})

const notAwaited =
  '[not awaited] locator.click at run:3:7 (op 9) was still running when the script returned; it was cancelled.'

test('run parser rejects the actual rendered unfinished-operation warning', () => {
  const reply = success(true)
  expect(() =>
    parseRunReply({
      ...reply,
      content: [{ type: 'text', text: `true\n${notAwaited}` }],
    }),
  ).toThrow('unfinished')
})

test('warning markers inside returned JSON or ordinary console entries are data', () => {
  const value = { note: notAwaited }
  expect(parseRunReply(success(value))).toMatchObject({ kind: 'ok', value })
  expect(parseRunReply(success(notAwaited))).toMatchObject({
    kind: 'ok',
    value: notAwaited,
  })
  expect(
    parseRunReply({
      ...success(true),
      content: [
        {
          type: 'text',
          text: 'true\nconsole:\nThe [not awaited] marker is ordinary console text.\n[not awaited] is also ordinary console text.',
        },
      ],
    }).kind,
  ).toBe('ok')
})

test('separate simultaneous runs initialize four fresh clients and distinct replay keys', async () => {
  let initialized = 0
  let calls = 0
  const requestIds: string[] = []
  const taskIds: string[] = []
  const allStarted = Promise.withResolvers<void>()
  const server = Bun.serve({
    port: 0,
    hostname: '127.0.0.1',
    async fetch(request) {
      const message = jsonObject(await request.json(), 'Test MCP request')
      if (message.method === 'initialize') {
        initialized += 1
        return Response.json({
          jsonrpc: '2.0',
          id: message.id,
          result: {
            protocolVersion: '2025-03-26',
            capabilities: {},
            serverInfo: { name: 'test', version: '1' },
          },
        })
      }
      if (message.method === 'notifications/initialized')
        return new Response(null, { status: 202 })
      const params = jsonObject(message.params, 'Test tool parameters')
      const meta = jsonObject(
        jsonObject(params._meta, 'Test metadata')['com.browseros.neo/run'],
        'Test run metadata',
      )
      requestIds.push(stringField(meta, 'requestId'))
      taskIds.push(stringField(meta, 'taskId'))
      calls += 1
      if (calls === 4) allStarted.resolve()
      await allStarted.promise
      return Response.json({
        jsonrpc: '2.0',
        id: message.id,
        result: success(true),
      })
    },
  })
  try {
    const run = createRunClient(`http://127.0.0.1:${server.port}`)
    const replies = await Promise.all(
      [0, 1, 2, 3].map((index) =>
        run({
          code: 'return true;',
          timeoutMs: 1000,
          taskId: 'same-task',
          requestId: `separate-${index}`,
        }),
      ),
    )
    expect(initialized).toBe(4)
    expect(new Set(requestIds).size).toBe(4)
    expect(new Set(taskIds)).toEqual(new Set(['same-task']))
    expect(new Set(replies.map((reply) => reply.runId)).size).toBe(4)
  } finally {
    await server.stop(true)
  }
})

test('a short independent driver deadline preserves the actual 600000ms SDK argument', async () => {
  const received: number[] = []
  const server = Bun.serve({
    port: 0,
    hostname: '127.0.0.1',
    async fetch(request) {
      const message = jsonObject(await request.json(), 'Driver request')
      if (message.method === 'notifications/initialized')
        return new Response(null, { status: 202 })
      if (message.method === 'initialize')
        return Response.json({
          jsonrpc: '2.0',
          id: message.id,
          result: {
            protocolVersion: '2025-03-26',
            capabilities: {},
            serverInfo: { name: 'test', version: '1' },
          },
        })
      const params = jsonObject(message.params, 'Run parameters')
      const args = jsonObject(params.arguments, 'Run arguments')
      if (typeof args.timeout !== 'number')
        throw new Error('Missing SDK timeout')
      received.push(args.timeout)
      return Response.json({
        jsonrpc: '2.0',
        id: message.id,
        result: success({ accepted: true }),
      })
    },
  })
  try {
    const reply = await createRunClient(`http://127.0.0.1:${server.port}`)({
      code: 'return {accepted:true};',
      timeoutMs: 600_000,
      driverTimeoutMs: 1_000,
      taskId: 'same-task',
      requestId: 'bounded-budget',
    })
    expect(reply).toMatchObject({ kind: 'ok', value: { accepted: true } })
    expect(received).toEqual([600_000])
  } finally {
    await server.stop(true)
  }
})

for (const stalled of [
  'initialize',
  'notifications/initialized',
  'tools/call',
]) {
  test(`the driver deadline bounds ${stalled} and late initialization cannot send a run`, async () => {
    const release = Promise.withResolvers<void>()
    const stalledStarted = Promise.withResolvers<void>()
    const released = Promise.withResolvers<void>()
    let runs = 0
    const server = Bun.serve({
      port: 0,
      hostname: '127.0.0.1',
      async fetch(request) {
        const message = jsonObject(await request.json(), 'Driver request')
        if (message.method === 'tools/call') runs += 1
        if (message.method === stalled) {
          stalledStarted.resolve()
          await release.promise
          released.resolve()
        }
        if (message.method === 'notifications/initialized')
          return new Response(null, { status: 202 })
        return Response.json({
          jsonrpc: '2.0',
          id: message.id,
          result:
            message.method === 'initialize'
              ? {
                  protocolVersion: '2025-03-26',
                  capabilities: {},
                  serverInfo: { name: 'test', version: '1' },
                }
              : success(true),
        })
      },
    })
    try {
      const pending = createRunClient(`http://127.0.0.1:${server.port}`)({
        code: 'return true;',
        timeoutMs: 600_000,
        driverTimeoutMs: 500,
        taskId: 'deadline-task',
        requestId: `bounded-${stalled}`,
      })
      await stalledStarted.promise
      await expect(pending).rejects.toThrow(
        'Independent driver deadline expired',
      )
      // Rejection joins the cancelled client attempt, so releasing the server cannot
      // resume an initialization or notification and submit a late tools/call.
      release.resolve()
      await released.promise
      expect(runs).toBe(stalled === 'tools/call' ? 1 : 0)
    } finally {
      release.resolve()
      await server.stop(true)
    }
  })
}

test('driver expiry joins session cleanup and retains its failure', async () => {
  const expired = Promise.withResolvers<void>()
  const cleanup = Promise.withResolvers<Response>()
  let posts = 0
  let deletes = 0
  const mockFetch: typeof fetch = Object.assign(
    async (
      _input: Parameters<typeof fetch>[0],
      init?: Parameters<typeof fetch>[1],
    ) => {
      if (init?.method === 'DELETE') {
        deletes += 1
        return await cleanup.promise
      }
      posts += 1
      init?.signal?.addEventListener('abort', () => expired.resolve(), {
        once: true,
      })
      return Response.json(
        { jsonrpc: '2.0', id: 1, result: {} },
        {
          headers: { 'mcp-session-id': 'deadline-cleanup-session' },
        },
      )
    },
    { preconnect: globalThis.fetch.preconnect },
  )
  const mock = spyOn(globalThis, 'fetch').mockImplementation(mockFetch)
  const pending = createRunClient('http://bounded-driver.invalid')({
    code: 'return true;',
    timeoutMs: 600_000,
    driverTimeoutMs: 500,
    taskId: 'deadline-task',
    requestId: 'bounded-cleanup-failure',
  })
  try {
    await expired.promise
    cleanup.resolve(new Response(null, { status: 500 }))
    await expect(pending).rejects.toThrow('Independent driver deadline expired')
    await expect(pending).rejects.toThrow(
      'Unexpected session cleanup failed: 500',
    )
    expect({ posts, deletes }).toEqual({ posts: 1, deletes: 1 })
  } finally {
    cleanup.resolve(new Response(null, { status: 204 }))
    await pending.catch(() => {})
    mock.mockRestore()
  }
})

for (const cleanupStatus of [204, 500]) {
  test(`the bounded driver closes stateful initialization with HTTP ${cleanupStatus} and never submits a run`, async () => {
    let closed = 0
    let runs = 0
    const server = Bun.serve({
      port: 0,
      hostname: '127.0.0.1',
      async fetch(request) {
        if (request.method === 'DELETE') {
          expect(request.headers.get('mcp-session-id')).toBe(
            'unexpected-test-session',
          )
          closed += 1
          return new Response(null, { status: cleanupStatus })
        }
        const message = jsonObject(await request.json(), 'Driver request')
        if (message.method === 'tools/call') runs += 1
        return Response.json(
          { jsonrpc: '2.0', id: message.id, result: {} },
          { headers: { 'mcp-session-id': 'unexpected-test-session' } },
        )
      },
    })
    try {
      const pending = createRunClient(`http://127.0.0.1:${server.port}`)({
        code: 'return true;',
        timeoutMs: 600_000,
        driverTimeoutMs: 1_000,
        taskId: 'deadline-task',
        requestId: 'bounded-stateful',
      })
      await expect(pending).rejects.toThrow('stateful MCP session')
      if (cleanupStatus === 500) {
        await expect(pending).rejects.toBeInstanceOf(AggregateError)
        await expect(pending).rejects.toThrow(
          'Unexpected session cleanup failed: 500',
        )
      }
      expect({ closed, runs }).toEqual({ closed: 1, runs: 0 })
    } finally {
      await server.stop(true)
    }
  })
}
