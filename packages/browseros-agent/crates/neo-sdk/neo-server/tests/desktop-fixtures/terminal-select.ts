/**
 * The sole terminal contract observes one unknown select operation in a disposable browser.
 * The uncertain observation never becomes an ordinary RunReply or passing CaseResult.
 * Cleanup keeps the strict decoder. Only the launcher parent can finalize pending evidence
 * after SDK, fixture, browser and private-state cleanup.
 */

import { randomUUID } from 'node:crypto'
import { CallToolResultSchema } from '@modelcontextprotocol/sdk/types.js'
import { McpSession } from '../../../../../contracts/claw-mcp/tests/mcp-client'
import { requireCondition } from './cases/requirements'
import {
  canonicalJson,
  jsonObject,
  jsonValue,
  numberField,
  stringField,
} from './json'
import { createRunClient, parseRunReply } from './mcp'
import {
  type CaseResult,
  type HostClient,
  isRunId,
  type JsonObject,
  type JsonValue,
  type RuledCaseSpec,
  type RunId,
} from './model'

const META = 'com.browseros.neo/run'
type SelectSpec = Extract<RuledCaseSpec, { execution: 'terminal-select' }>

/** No status:pass field can exist until the parent finishes the enclosing launcher. */
export interface PendingSelect {
  kind: 'pending-select'
  id: 'act-017'
  file: 'cases-act.ts'
  name: string
  taskId: string
  requestId: string
  durationMs: number
  cleanupMs: number
  runIds: RunId[]
  raw: JsonValue
  taskCleanup: JsonValue
  settingsRestored: true
}

export type TerminalCheck =
  | { kind: 'pending'; observation: PendingSelect }
  | { kind: 'failed'; result: CaseResult & { status: 'fail' } }

function exactKeys(
  value: JsonObject,
  keys: readonly string[],
  label: string,
): void {
  requireCondition(
    Object.keys(value).sort().join(',') === [...keys].sort().join(','),
    `Unexpected ${label} fields`,
  )
}

function same(
  value: JsonValue | undefined,
  expected: JsonValue,
  label: string,
): void {
  requireCondition(
    value !== undefined && canonicalJson(value) === canonicalJson(expected),
    `Invalid terminal select ${label}`,
  )
}

/** Closed parser: the same envelope still fails the unchanged ordinary parseRunReply. */
function selectObservation(
  input: unknown,
  identity: { taskId: string; requestId: string },
): RunId {
  const result = CallToolResultSchema.parse(input)
  requireCondition(
    result.isError === undefined || result.isError === false,
    'Terminal select cannot report an MCP error',
  )
  requireCondition(
    result.content.every((block) => block.type === 'text'),
    'Terminal select must contain text only',
  )
  requireCondition(
    result.content.length === 1,
    'Terminal select must have one canonical text block',
  )
  const meta = jsonObject(result._meta?.[META], 'Terminal select metadata')
  const runId = meta.id
  requireCondition(isRunId(runId), 'Terminal select omitted a valid run ID')
  requireCondition(
    meta.status === 'ok' && meta.effects === 'unknown',
    'Terminal select requires status ok and unknown effects',
  )
  requireCondition(
    meta.error === undefined,
    'Terminal select cannot carry a metadata error',
  )
  requireCondition(
    meta.tabsScope === 'task',
    'Terminal select must stay in its bound task',
  )
  const structured = jsonObject(
    result.structuredContent,
    'Terminal select structured result',
  )
  exactKeys(structured, ['value'], 'terminal structured result')
  const value = jsonObject(structured.value, 'Terminal select value')
  exactKeys(
    value,
    ['selected', 'before', 'after', 'error', 'taskId', 'requestId'],
    'terminal select value',
  )
  same(value.selected, ['green'], 'selected values')
  same(
    value.before,
    { value: 'green', changes: 1, prompt: 'Green' },
    'pre-timeout state',
  )
  same(
    value.after,
    { value: 'green', changes: 1, prompt: 'Green' },
    'post-timeout state',
  )
  requireCondition(
    value.taskId === identity.taskId && value.requestId === identity.requestId,
    'Terminal select request/task correlation differs',
  )
  const error = jsonObject(value.error, 'Caught terminal select error')
  exactKeys(
    error,
    [
      'neo',
      'name',
      'owner',
      'code',
      'delivery',
      'operationId',
      'timeout',
      'message',
    ],
    'caught select error',
  )
  requireCondition(
    error.neo === true &&
      error.name === 'TimeoutError' &&
      error.timeout === true,
    'Terminal select must retain NeoError timeout identity',
  )
  requireCondition(
    error.owner === 'website' &&
      error.code === 'not_actionable' &&
      error.delivery === 'unknown',
    'Terminal select caught error contract differs',
  )
  const message = stringField(error, 'message')
  requireCondition(
    /^locator\.selectOption: Timeout 1500ms exceeded\./.test(message),
    'Terminal select timeout message differs',
  )
  const operationId = stringField(error, 'operationId')
  requireCondition(
    operationId.startsWith(`${runId}.`),
    'Terminal select operation belongs to another run',
  )
  const sequence = operationId.slice(runId.length + 1)
  requireCondition(
    /^[1-9]\d*$/.test(sequence) && Number.isSafeInteger(Number(sequence)),
    'Terminal select requires a positive operation sequence',
  )
  same(
    meta.unknown,
    [{ operationId, name: 'locator.selectOption' }],
    'sole unknown operation',
  )
  const first = result.content[0]
  if (first?.type !== 'text') throw new Error('Terminal select omitted text')
  const lines = first.text.split('\n')
  requireCondition(
    lines.length === 2 && lines[0] === JSON.stringify(value),
    'Terminal select text must contain only its value and one uncertainty note',
  )
  const note = new RegExp(
    `^\\[unknown outcome\\] locator\\.selectOption(?: at run:\\d+:\\d+)? \\(op ${sequence}\\) was sent but not confirmed\\. Take a snapshot before repeating it\\.$`,
  )
  requireCondition(
    note.test(lines[1] ?? ''),
    'Terminal select uncertainty note differs from its operation',
  )
  return runId
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** Executes exactly this one observation, then permits only task/settings cleanup. */
export async function observeMissingOptionSelect(options: {
  spec: SelectSpec
  baseUrl: string
  base: string
  base2: string
  common: string
  body: string
  host: HostClient
}): Promise<TerminalCheck> {
  const started = performance.now()
  const taskId = `desktop-act-017-${randomUUID()}`
  const requestId = `desktop-${randomUUID()}`
  const identity = { taskId, requestId }
  let raw: JsonValue | undefined
  let taskCleanup: JsonValue | undefined
  let runId: RunId | undefined
  let savedSettings: JsonObject | undefined
  let failure: string | undefined
  let settingsRestored = false
  try {
    savedSettings = await options.host.settings()
    const client = await McpSession.connect(
      options.baseUrl,
      'sdk-desktop-fixtures-terminal-select',
    )
    if (client.sessionId !== undefined) {
      await client.close()
      throw new Error('Source SDK unexpectedly opened a stateful MCP session')
    }
    const result = await client.request(
      'tools/call',
      {
        name: 'run',
        arguments: {
          code: `const args = ${JSON.stringify({ ...identity, base: options.base, base2: options.base2 })};\n${options.common}\n${options.body}`,
          timeout: options.spec.timeoutMs,
        },
        _meta: {
          [META]: { ...identity, hostData: { suite: 'sdk-desktop-fixtures' } },
        },
      },
      options.spec.timeoutMs + 30_000,
    )
    raw = jsonValue(result)
    runId = selectObservation(result, identity)
  } catch (error) {
    failure = message(error)
  }
  const cleanupStarted = performance.now()
  try {
    const reply = await createRunClient(options.baseUrl)({
      taskId,
      requestId: `desktop-cleanup-${randomUUID()}`,
      timeoutMs: 20_000,
      code: `await (await neo.task(${JSON.stringify(taskId)})).close(); return true;`,
    })
    requireCondition(
      reply.kind === 'ok' && reply.value === true,
      'Terminal task close did not return true',
    )
    taskCleanup = reply.raw
    requireClosedTask(taskCleanup, taskId)
  } catch (error) {
    failure = [failure, `Task cleanup: ${message(error)}`]
      .filter(Boolean)
      .join('; ')
  }
  if (savedSettings) {
    try {
      await options.host.restoreSettings(savedSettings)
      same(await options.host.settings(), savedSettings, 'restored settings')
      settingsRestored = true
    } catch (error) {
      failure = [failure, `Settings restoration: ${message(error)}`]
        .filter(Boolean)
        .join('; ')
    }
  }
  const base = {
    id: options.spec.id,
    file: options.spec.file,
    name: options.spec.name,
    durationMs: cleanupStarted - started,
    cleanupMs: performance.now() - cleanupStarted,
    runIds: runId ? [runId] : [],
  }
  if (
    failure ||
    raw === undefined ||
    taskCleanup === undefined ||
    !settingsRestored
  ) {
    return {
      kind: 'failed',
      result: {
        ...base,
        status: 'fail',
        failure: failure ?? 'Terminal select omitted required evidence',
      },
    }
  }
  return {
    kind: 'pending',
    observation: {
      ...base,
      kind: 'pending-select',
      ...identity,
      raw,
      taskCleanup,
      settingsRestored: true,
    },
  }
}

function requireClosedTask(input: unknown, taskId: string): void {
  const reply = parseRunReply(input)
  requireCondition(
    reply.kind === 'ok' && reply.value === true,
    'Terminal task cleanup reply failed',
  )
  const meta = jsonObject(
    jsonObject(
      jsonObject(reply.raw, 'Cleanup reply')._meta,
      'Cleanup metadata',
    )[META],
    'Cleanup run metadata',
  )
  requireCondition(
    Array.isArray(meta.tasks),
    'Terminal cleanup omitted task records',
  )
  const closed = meta.tasks.filter(
    (task) => jsonObject(task, 'Cleanup task').id === taskId,
  )
  requireCondition(
    closed.length === 1 &&
      jsonObject(closed[0], 'Closed task').kind === 'closed',
    'Terminal cleanup did not close its exact task',
  )
}

/** The parent revalidates transferred evidence; this returns pending data, never a pass. */
export function readPendingSelect(
  input: unknown,
  spec: SelectSpec,
): PendingSelect {
  const value = jsonObject(input, 'Pending select observation')
  exactKeys(
    value,
    [
      'kind',
      'id',
      'file',
      'name',
      'taskId',
      'requestId',
      'durationMs',
      'cleanupMs',
      'runIds',
      'raw',
      'taskCleanup',
      'settingsRestored',
    ],
    'pending observation',
  )
  requireCondition(
    value.kind === 'pending-select' &&
      value.id === spec.id &&
      value.file === spec.file &&
      value.name === spec.name &&
      value.settingsRestored === true,
    'Pending select identity or settings cleanup differs',
  )
  const taskId = stringField(value, 'taskId')
  const requestId = stringField(value, 'requestId')
  requireCondition(
    /^desktop-act-017-[a-f0-9-]+$/.test(taskId) &&
      /^desktop-[a-f0-9-]+$/.test(requestId),
    'Pending select omitted owned request/task identities',
  )
  const durationMs = numberField(value, 'durationMs')
  const cleanupMs = numberField(value, 'cleanupMs')
  requireCondition(
    Number.isFinite(durationMs) &&
      durationMs >= 0 &&
      Number.isFinite(cleanupMs) &&
      cleanupMs >= 0,
    'Pending select timings must be finite and nonnegative',
  )
  const runId = selectObservation(value.raw, { taskId, requestId })
  same(value.runIds, [runId], 'pending run IDs')
  requireClosedTask(value.taskCleanup, taskId)
  return {
    kind: 'pending-select',
    id: 'act-017',
    file: 'cases-act.ts',
    name: spec.name,
    taskId,
    requestId,
    durationMs,
    cleanupMs,
    runIds: [runId],
    raw: jsonValue(value.raw),
    taskCleanup: jsonValue(value.taskCleanup),
    settingsRestored: true,
  }
}
