import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { Harness } from './harness'
import { jsonObject } from './json'
import { parseRunReply } from './mcp'
import type {
  CaseSpec,
  HostClient,
  JsonObject,
  JsonValue,
  RunRequest,
} from './model'

function ok(value: JsonValue) {
  return parseRunReply({
    content: [{ type: 'text', text: JSON.stringify(value) }],
    structuredContent: { value },
    _meta: { 'com.browseros.neo/run': { id: randomUUID(), status: 'ok' } },
  })
}

function failed(message: string, owner = 'script', code = 'script_error') {
  return parseRunReply({
    content: [{ type: 'text', text: message }],
    structuredContent: { error: { message, owner, code } },
    isError: true,
    _meta: { 'com.browseros.neo/run': { id: randomUUID(), status: 'error' } },
  })
}

const basic = {
  id: 'unit-001',
  file: 'cases-unit.ts',
  name: 'Unit contract',
  proves: 'Owned lifecycle',
  steps: [{ kind: 'script', script: 'body.txt' }],
  timeoutMs: 1000,
  expectation: { kind: 'pass' },
} satisfies CaseSpec

function hostClient() {
  let current: JsonObject = {
    tabs: { scope: 'task' },
    trace: { valueBytes: 0 },
    sentinel: 'preserved',
  }
  const events: string[] = []
  const host: HostClient = {
    async settings() {
      return structuredClone(current)
    },
    async patchSettings(patch) {
      events.push('patch')
      current = { ...current, ...patch }
      return structuredClone(current)
    },
    async restoreSettings(settings) {
      events.push('restore')
      current = structuredClone(settings)
      return structuredClone(current)
    },
    async record() {
      throw new Error('Unexpected host record')
    },
    async blob() {
      throw new Error('Unexpected host blob')
    },
    async download() {
      throw new Error('Unexpected host download')
    },
  }
  return { host, events }
}

const targets = {
  async create() {
    return 'external-target'
  },
  async close() {},
}
const environment = {
  base: 'http://127.0.0.1:18080',
  base2: 'http://localhost:18081',
  scale: 1,
  common: '',
  async loadBody(script: string) {
    return script
  },
  targets,
} satisfies Partial<ConstructorParameters<typeof Harness>[0]>

function argsOf(request: RunRequest) {
  return jsonObject(
    JSON.parse(
      request.code.slice('const args = '.length, request.code.indexOf(';\n')),
    ),
    'Script args',
  )
}

test('parallel steps overlap as separate requests and preserve ordered prior values', async () => {
  const calls: RunRequest[] = []
  const allStarted = Promise.withResolvers<void>()
  let captures = 0
  const state = hostClient()
  const harness = new Harness({
    ...environment,
    host: state.host,
    async run(request) {
      calls.push(request)
      if (request.code.startsWith('await (await neo.task(')) return ok(true)
      const args = argsOf(request)
      if (request.code.endsWith('prepare.txt'))
        return ok({ baseline: 'full snapshot' })
      if (request.code.endsWith('capture.txt')) {
        const index = captures++
        if (captures === 4) allStarted.resolve()
        await allStarted.promise
        expect(args.prior).toEqual([{ baseline: 'full snapshot' }])
        return ok(index)
      }
      expect(args.prior).toEqual([{ baseline: 'full snapshot' }, [0, 1, 2, 3]])
      return ok(true)
    },
  })
  const spec = {
    ...basic,
    steps: [
      { kind: 'script', script: 'prepare.txt' },
      {
        kind: 'parallel',
        scripts: ['capture.txt', 'capture.txt', 'capture.txt', 'capture.txt'],
      },
      { kind: 'script', script: 'finish.txt' },
    ],
  } satisfies CaseSpec
  const result = await harness.check(spec)
  expect(result.status).toBe('pass')
  expect(calls).toHaveLength(7)
  expect(new Set(calls.map((call) => call.taskId)).size).toBe(1)
  expect(new Set(calls.map((call) => call.requestId)).size).toBe(7)
  expect(new Set(result.runIds).size).toBe(6)
})

test('a parallel transport rejection waits for sibling calls before task cleanup', async () => {
  const allStarted = Promise.withResolvers<void>()
  let started = 0
  let completed = 0
  const state = hostClient()
  const harness = new Harness({
    ...environment,
    host: state.host,
    async run(request) {
      if (request.code.startsWith('await (await neo.task(')) {
        expect(completed).toBe(3)
        return ok(true)
      }
      const index = started++
      if (started === 4) allStarted.resolve()
      await allStarted.promise
      if (index === 0) throw new Error('Transport closed')
      await Bun.sleep(5)
      completed += 1
      return ok(index)
    },
  })
  const result = await harness.check({
    ...basic,
    steps: [
      { kind: 'parallel', scripts: ['a.txt', 'a.txt', 'a.txt', 'a.txt'] },
    ],
  })
  expect(result).toMatchObject({
    status: 'fail',
    failure: 'Parallel MCP transport failed: Transport closed',
  })
  expect(harness.poisoned).toBe(true)
  await expect(harness.check(basic)).rejects.toThrow(
    'prevents further browser state reuse',
  )
})

test('cleanup attempts borrowed target, named task and settings after a matching xfail', async () => {
  const state = hostClient()
  const events: string[] = []
  const harness = new Harness({
    ...environment,
    host: state.host,
    targets: {
      async create() {
        events.push('create')
        return 'only-owned-target'
      },
      async close(id) {
        events.push(`close:${id}`)
      },
    },
    async run(request) {
      if (request.code.startsWith('await (await neo.task(')) {
        events.push('task-close')
        return failed('cleanup refused', 'host', 'cleanup_failure')
      }
      return failed(
        'Error: invalid fill leaves the field and application event record unchanged; expected={"value":"2020-01-02","events":[]}; actual={"value":"","events":[]} (run:5:7)',
      )
    },
  })
  const result = await harness.check({
    ...basic,
    expectation: {
      kind: 'xfail',
      reason: 'known date reset',
      signature: 'invalid-date-reset',
    },
    settings: { tabs: { scope: 'browser' } },
    hostSetup: { kind: 'borrowed-page', urlTemplate: '{base}/upload.html' },
  })
  expect(result.status).toBe('fail')
  expect(events).toEqual(['create', 'close:only-owned-target', 'task-close'])
  expect(state.events).toEqual(['patch', 'restore'])
  expect(await state.host.settings()).toEqual({
    tabs: { scope: 'task' },
    trace: { valueBytes: 0 },
    sentinel: 'preserved',
  })
  expect(harness.poisoned).toBe(true)
  await expect(harness.check(basic)).rejects.toThrow(
    'prevents further browser state reuse',
  )
})

test('settings setup errors fail normally, still restore and cannot become xfails', async () => {
  const state = hostClient()
  const runCalls: string[] = []
  const harness = new Harness({
    ...environment,
    host: {
      ...state.host,
      async patchSettings() {
        throw new Error('setup unavailable')
      },
    },
    async run(request) {
      runCalls.push(request.code)
      return ok(true)
    },
  })
  const result = await harness.check({
    ...basic,
    settings: { trace: { valueBytes: 262144 } },
    expectation: {
      kind: 'xfail',
      reason: 'known date reset',
      signature: 'invalid-date-reset',
    },
  })
  expect(result).toMatchObject({ status: 'fail', failure: 'setup unavailable' })
  expect(runCalls).toHaveLength(1)
  expect(state.events).toEqual(['restore'])
  expect(harness.poisoned).toBe(false)
})

test('uncertain borrowed-target creation stops reuse after named-task cleanup', async () => {
  const state = hostClient()
  const calls: string[] = []
  const harness = new Harness({
    ...environment,
    host: state.host,
    targets: {
      async create() {
        throw new Error('Root CDP reply lost')
      },
      async close() {
        throw new Error('No target ID was returned')
      },
    },
    async run(request) {
      calls.push(request.code)
      return ok(true)
    },
  })
  const result = await harness.check({
    ...basic,
    hostSetup: { kind: 'borrowed-page', urlTemplate: '{base}/upload.html' },
    expectation: {
      kind: 'xfail',
      reason: 'known failure',
      signature: 'invalid-date-reset',
    },
  })
  expect(result).toMatchObject({
    status: 'fail',
    failure: 'Root CDP reply lost',
  })
  expect(calls).toHaveLength(1)
  expect(harness.poisoned).toBe(true)
  await expect(harness.check(basic)).rejects.toThrow(
    'prevents further browser state reuse',
  )
})

const invalidDateFailure =
  'Error: invalid fill leaves the field and application event record unchanged; expected={"value":"2020-01-02","events":[]}; actual={"value":"","events":[]} (run:5:7)'
const uncertainReplies: { name: string; spec: CaseSpec; raw: JsonObject }[] = [
  {
    name: 'caught write error with status ok and unknown effects',
    spec: basic,
    raw: {
      content: [{ type: 'text', text: '{"caught":true}' }],
      structuredContent: { value: { caught: true } },
      _meta: {
        'com.browseros.neo/run': {
          id: randomUUID(),
          status: 'ok',
          effects: 'unknown',
          unknown: [],
        },
      },
    },
  },
  {
    name: 'caught write error with status ok and an unconfirmed operation',
    spec: basic,
    raw: {
      content: [{ type: 'text', text: '{"caught":true}' }],
      structuredContent: { value: { caught: true } },
      _meta: {
        'com.browseros.neo/run': {
          id: randomUUID(),
          status: 'ok',
          effects: 'done',
          unknown: [
            { operationId: `${randomUUID()}.1`, name: 'locator.click' },
          ],
        },
      },
    },
  },
  {
    name: 'known xfail followed by a rendered unfinished operation',
    spec: {
      ...basic,
      expectation: {
        kind: 'xfail',
        reason: 'known date reset',
        signature: 'invalid-date-reset',
      },
    },
    raw: {
      content: [
        {
          type: 'text',
          text: `script / script_error: ${invalidDateFailure}\n[not awaited] locator.click (op 1) was still running when the script returned; it was cancelled.`,
        },
      ],
      structuredContent: {
        error: {
          owner: 'script',
          code: 'script_error',
          message: invalidDateFailure,
        },
      },
      isError: true,
      _meta: {
        'com.browseros.neo/run': {
          id: randomUUID(),
          status: 'error',
          effects: 'done',
          unknown: [],
        },
      },
    },
  },
]

for (const scenario of uncertainReplies) {
  test(`parser uncertainty poisons reuse after named cleanup: ${scenario.name}`, async () => {
    const state = hostClient()
    const calls: RunRequest[] = []
    const harness = new Harness({
      ...environment,
      host: state.host,
      async run(request) {
        calls.push(request)
        if (request.code.startsWith('await (await neo.task(')) return ok(true)
        return parseRunReply(scenario.raw)
      },
    })
    const result = await harness.check(scenario.spec)
    expect(result.status).toBe('fail')
    expect(harness.poisoned).toBe(true)
    expect(calls).toHaveLength(2)
    const [body, cleanup] = calls
    if (!body || !cleanup)
      throw new Error('The case must attempt body and cleanup')
    expect(cleanup.code).toBe(
      `await (await neo.task(${JSON.stringify(body.taskId)})).close(); return true;`,
    )
    expect(cleanup.taskId).toBe(body.taskId)
    expect(cleanup.requestId).not.toBe(body.requestId)
    await expect(harness.check(basic)).rejects.toThrow(
      'prevents further browser state reuse',
    )
    expect(calls).toHaveLength(2)
  })
}
