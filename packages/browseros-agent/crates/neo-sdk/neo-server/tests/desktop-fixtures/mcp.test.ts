import { expect, test } from 'bun:test'
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
