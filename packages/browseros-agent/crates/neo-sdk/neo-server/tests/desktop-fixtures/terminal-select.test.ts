import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { jsonObject } from './json'
import { parseRunReply } from './mcp'
import { isRunId, type JsonObject } from './model'
import { ruledCases } from './ruled/cases'
import { finalizeTerminalPartition } from './run'
import { type PendingSelect, readPendingSelect } from './terminal-select'

const META = 'com.browseros.neo/run'
function terminalSpec() {
  const spec = ruledCases.find((entry) => entry.execution === 'terminal-select')
  if (!spec || spec.execution !== 'terminal-select')
    throw new Error('Missing terminal select rule')
  return spec
}

/** Synthetic protocol data exercises rejection boundaries; live browser acceptance is separate. */
function pending(): PendingSelect {
  const id = randomUUID()
  if (!isRunId(id)) throw new Error('Invalid generated run ID')
  const taskId = `desktop-act-017-${randomUUID()}`
  const requestId = `desktop-${randomUUID()}`
  const operationId = `${id}.7`
  const state = { value: 'green', changes: 1, prompt: 'Green' }
  const value = {
    selected: ['green'],
    before: state,
    after: state,
    taskId,
    requestId,
    error: {
      neo: true,
      name: 'TimeoutError',
      owner: 'website',
      code: 'not_actionable',
      delivery: 'unknown',
      operationId,
      timeout: true,
      message: 'locator.selectOption: Timeout 1500ms exceeded.',
    },
  }
  return {
    kind: 'pending-select',
    id: 'act-017',
    file: 'cases-act.ts',
    name: terminalSpec().name,
    taskId,
    requestId,
    durationMs: 10,
    cleanupMs: 2,
    runIds: [id],
    settingsRestored: true,
    raw: {
      content: [
        {
          type: 'text',
          text: `${JSON.stringify(value)}\n[unknown outcome] locator.selectOption at run:10:3 (op 7) was sent but not confirmed. Take a snapshot before repeating it.`,
        },
      ],
      structuredContent: { value },
      _meta: {
        [META]: {
          id,
          status: 'ok',
          effects: 'unknown',
          unknown: [{ operationId, name: 'locator.selectOption' }],
          tasks: [],
          tabsScope: 'task',
        },
      },
    },
    taskCleanup: {
      content: [{ type: 'text', text: 'true' }],
      structuredContent: { value: true },
      _meta: {
        [META]: {
          id: randomUUID(),
          status: 'ok',
          effects: 'done',
          unknown: [],
          tasks: [{ id: taskId, kind: 'closed', name: null, tabs: [] }],
          tabsScope: 'task',
        },
      },
    },
  }
}

function patchMeta(observation: PendingSelect, patch: JsonObject): void {
  const raw = jsonObject(observation.raw, 'Test reply')
  const meta = jsonObject(
    jsonObject(raw._meta, 'Test metadata')[META],
    'Test run metadata',
  )
  observation.raw = { ...raw, _meta: { [META]: { ...meta, ...patch } } }
}

function patchValue(observation: PendingSelect, patch: JsonObject): void {
  const raw = jsonObject(observation.raw, 'Test reply')
  const value = jsonObject(
    jsonObject(raw.structuredContent, 'Test structured').value,
    'Test value',
  )
  observation.raw = {
    ...raw,
    structuredContent: { value: { ...value, ...patch } },
  }
}

test('the closed select observation stays pending and the ordinary parser still rejects it', () => {
  const observation = pending()
  expect(readPendingSelect(observation, terminalSpec()).kind).toBe(
    'pending-select',
  )
  expect(Object.hasOwn(observation, 'status')).toBe(false)
  expect(() => parseRunReply(observation.raw)).toThrow(
    'uncertain write effects',
  )
})

const corruptions: readonly [string, (observation: PendingSelect) => void][] = [
  ['wrong effects', (value) => patchMeta(value, { effects: 'done' })],
  ['wrong status', (value) => patchMeta(value, { status: 'error' })],
  [
    'metadata error',
    (value) => patchMeta(value, { error: { code: 'script_error' } }),
  ],
  ['wrong scope', (value) => patchMeta(value, { tabsScope: 'browser' })],
  ['missing unknown', (value) => patchMeta(value, { unknown: [] })],
  [
    'wrong unknown operation',
    (value) =>
      patchMeta(value, {
        unknown: [
          { name: 'locator.click', operationId: `${value.runIds[0]}.7` },
        ],
      }),
  ],
  [
    'foreign run operation',
    (value) =>
      patchMeta(value, {
        unknown: [
          { name: 'locator.selectOption', operationId: `${randomUUID()}.7` },
        ],
      }),
  ],
  [
    'extra unknown operation',
    (value) =>
      patchMeta(value, {
        unknown: [
          { name: 'locator.selectOption', operationId: `${value.runIds[0]}.7` },
          { name: 'locator.click', operationId: `${value.runIds[0]}.8` },
        ],
      }),
  ],
  [
    'changed DOM state',
    (value) =>
      patchValue(value, { after: { value: 'red', changes: 2, prompt: 'Red' } }),
  ],
  [
    'wrong task correlation',
    (value) => patchValue(value, { taskId: 'another-task' }),
  ],
  [
    'wrong request correlation',
    (value) => patchValue(value, { requestId: 'another-request' }),
  ],
  ['missing selected return', (value) => patchValue(value, { selected: [] })],
  [
    'MCP error',
    (value) => {
      value.raw = { ...jsonObject(value.raw, 'Test reply'), isError: true }
    },
  ],
  [
    'image block',
    (value) => {
      const raw = jsonObject(value.raw, 'Test reply')
      value.raw = {
        ...raw,
        content: [{ type: 'image', data: '', mimeType: 'image/png' }],
      }
    },
  ],
  [
    'unfinished work',
    (value) => {
      const raw = jsonObject(value.raw, 'Test reply')
      if (!Array.isArray(raw.content)) throw new Error('Missing test content')
      const first = jsonObject(raw.content[0], 'Test text')
      value.raw = {
        ...raw,
        content: [
          {
            ...first,
            text: `${first.text}\n[not awaited] locator.click (op 8) was still running when the script returned; it was cancelled.`,
          },
        ],
      }
    },
  ],
  [
    'unrelated note',
    (value) => {
      const raw = jsonObject(value.raw, 'Test reply')
      if (!Array.isArray(raw.content)) throw new Error('Missing test content')
      const first = jsonObject(raw.content[0], 'Test text')
      value.raw = {
        ...raw,
        content: [
          { ...first, text: `${first.text}\nTip: unrelated uncertainty` },
        ],
      }
    },
  ],
  [
    'uncertain cleanup',
    (value) => {
      const raw = jsonObject(value.taskCleanup, 'Cleanup')
      const meta = jsonObject(
        jsonObject(raw._meta, 'Cleanup metadata')[META],
        'Cleanup run',
      )
      value.taskCleanup = {
        ...raw,
        _meta: { [META]: { ...meta, effects: 'unknown' } },
      }
    },
  ],
  [
    'wrong closed task',
    (value) => {
      const raw = jsonObject(value.taskCleanup, 'Cleanup')
      const meta = jsonObject(
        jsonObject(raw._meta, 'Cleanup metadata')[META],
        'Cleanup run',
      )
      value.taskCleanup = {
        ...raw,
        _meta: {
          [META]: { ...meta, tasks: [{ id: 'another-task', kind: 'closed' }] },
        },
      }
    },
  ],
]

for (const [name, corrupt] of corruptions) {
  test(`terminal observation rejects ${name}`, () => {
    const observation = pending()
    corrupt(observation)
    expect(() => readPendingSelect(observation, terminalSpec())).toThrow()
  })
}

const errorCorruptions: readonly JsonObject[] = [
  { neo: false },
  { name: 'Error' },
  { owner: 'script' },
  { code: 'invalid_input' },
  { delivery: 'done' },
  { timeout: false },
  { operationId: `${randomUUID()}.7` },
  { operationId: 'missing-sequence' },
  { message: 'some other timeout' },
]
for (const patch of errorCorruptions) {
  test(`terminal caught error rejects ${JSON.stringify(patch)}`, () => {
    const observation = pending()
    const raw = jsonObject(observation.raw, 'Test reply')
    const value = jsonObject(
      jsonObject(raw.structuredContent, 'Test structured').value,
      'Test value',
    )
    patchValue(observation, {
      error: { ...jsonObject(value.error, 'Test error'), ...patch },
    })
    expect(() => readPendingSelect(observation, terminalSpec())).toThrow()
  })
}

test('parent emits a named result only after checking actual cleanup artifacts and absent private state', async () => {
  const root = await mkdtemp(join(tmpdir(), 'sdk-terminal-proof-'))
  const out = join(root, 'out')
  const launcher = join(root, 'launcher')
  const ownedProfile = join(root, 'profile')
  await mkdir(out)
  await mkdir(launcher)
  const write = (path: string, value: unknown) =>
    writeFile(path, `${JSON.stringify(value)}\n`)
  const receipt = {
    partition: 'terminal-select',
    startedAt: 1,
    startupMs: 1,
    casesMs: 10,
    caseCleanupMs: 2,
    cleanupMs: 3,
    finishedAt: 20,
    expectedIds: ['act-017'],
    results: [],
    complete: false,
  }
  try {
    await write(join(out, 'receipt.json'), receipt)
    await write(join(out, 'pending-select.json'), {
      observation: pending(),
      sdkStopped: true,
      fixturesStopped: true,
      failure: null,
    })
    // These deliberately absent PIDs exercise receipt verification without launching a process.
    const owner = {
      pid: 2_147_483_647,
      profile: ownedProfile,
      home: join(root, 'removed-home'),
      temp: join(root, 'removed-temp'),
    }
    await write(join(out, 'browser.json'), {
      profile: owner.profile,
      home: owner.home,
    })
    await write(join(launcher, 'owner.json'), owner)
    await write(join(out, 'server.log.lifecycle.json'), {
      pid: 2_147_483_646,
      cleaned: true,
      forced: false,
      exitCode: 0,
      root: join(root, 'removed-sdk'),
      socketPath: join(root, 'removed-sdk/host.sock'),
    })
    await write(join(launcher, 'result.json'), {
      state: 'finished',
      code: 0,
      cleaned: true,
      cleanupErrors: [],
      command: { pid: 2_147_483_645 },
    })
    await write(join(launcher, 'cleanup.json'), owner)
    await mkdir(ownedProfile)
    await finalizeTerminalPartition({ out, runDir: launcher, code: 0 })
    expect(await readFile(join(out, 'junit.xml'), 'utf8')).toContain(
      '<failure message="Launcher left its profile behind"',
    )
    expect(
      jsonObject(
        JSON.parse(await readFile(join(out, 'receipt.json'), 'utf8')),
        'Final receipt',
      ).complete,
    ).toBe(false)
    await rm(ownedProfile, { recursive: true })
    await write(join(out, 'receipt.json'), receipt)
    await finalizeTerminalPartition({ out, runDir: launcher, code: 0 })
    const complete = jsonObject(
      JSON.parse(await readFile(join(out, 'receipt.json'), 'utf8')),
      'Final receipt',
    )
    expect(complete.complete).toBe(true)
    expect(complete.finishedAt).toBe(receipt.finishedAt)
    expect(typeof complete.finalizedAt).toBe('number')
    expect(complete.results).toMatchObject([{ id: 'act-017', status: 'pass' }])
    const xml = await readFile(join(out, 'junit.xml'), 'utf8')
    expect(xml).toContain(
      'name="act-017 act: select updates a covered styled native select semantically"',
    )
    expect(xml).not.toContain('<failure')
    await write(join(out, 'receipt.json'), receipt)
    await write(join(out, 'browser.json'), {
      profile: join(root, 'different-profile'),
      home: owner.home,
    })
    await finalizeTerminalPartition({ out, runDir: launcher, code: 0 })
    expect(await readFile(join(out, 'junit.xml'), 'utf8')).toContain(
      'belongs to another browser',
    )
    await write(join(out, 'browser.json'), {
      profile: owner.profile,
      home: owner.home,
    })
    await write(join(out, 'receipt.json'), receipt)
    await write(join(out, 'server.log.lifecycle.json'), {
      pid: process.pid,
      cleaned: true,
      forced: false,
      exitCode: 0,
      root: join(root, 'removed-sdk'),
      socketPath: join(root, 'removed-sdk/host.sock'),
    })
    await finalizeTerminalPartition({ out, runDir: launcher, code: 0 })
    expect(await readFile(join(out, 'junit.xml'), 'utf8')).toContain(
      'SDK process survived cleanup',
    )
    await write(join(out, 'receipt.json'), receipt)
    await finalizeTerminalPartition({ out, code: 0 })
    expect(await readFile(join(out, 'junit.xml'), 'utf8')).toContain(
      'omitted cleanup artifacts',
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
