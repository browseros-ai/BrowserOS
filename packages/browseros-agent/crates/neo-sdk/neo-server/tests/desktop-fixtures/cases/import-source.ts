import type {
  CaseSpec,
  HostCheck,
  HostSetup,
  JsonObject,
  JsonValue,
  SettingsPatch,
  Step,
} from '../model'
import { requireCondition, strictFailures } from './requirements'

export interface OriginalCase {
  id: string
  file: string
  name: string
}

export type Classification =
  | 'DIFFERENT BY DESIGN'
  | 'NO EQUIVALENT'
  | 'NOT A BROWSER BEHAVIOUR'
export type SavedStatus = Classification | 'PASS' | 'FAIL'
export type ManifestCase = OriginalCase &
  (
    | { kind: 'selected'; case: CaseSpec }
    | { kind: 'classified'; classification: Classification }
  )

/** Unknown JSON stops here. Only selected, normalized contracts can reach public serialization. */
function object(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Expected an object')
  return Object.fromEntries(Object.entries(value))
}

function text(value: unknown): string {
  requireCondition(typeof value === 'string', 'Expected a string')
  return value
}

function integer(value: unknown): number {
  requireCondition(
    typeof value === 'number' && Number.isSafeInteger(value) && value > 0,
    'Expected a positive integer',
  )
  return value
}

function array(value: unknown): unknown[] {
  requireCondition(Array.isArray(value), 'Expected an array')
  return value
}

function strings(value: unknown): string[] {
  return array(value).map(text)
}

function knownFields(
  row: Record<string, unknown>,
  allowed: readonly string[],
): void {
  requireCondition(
    Object.keys(row).every((field) => allowed.includes(field)),
    'Unknown source metadata must be modeled before it can be imported',
  )
}

function json(value: unknown): JsonValue {
  if (value === null || typeof value === 'string' || typeof value === 'boolean')
    return value
  if (typeof value === 'number') {
    requireCondition(Number.isFinite(value), 'JSON numbers must be finite')
    return value
  }
  if (Array.isArray(value)) return value.map(json)
  const result: JsonObject = {}
  for (const [key, item] of Object.entries(object(value)))
    result[key] = json(item)
  return result
}

export function parseOriginalCases(value: unknown): OriginalCase[] {
  return array(value).map((item) => {
    const row = object(item)
    requireCondition(
      Object.keys(row).sort().join(',') === 'file,id,name',
      'Original inventory rows must contain only file, id, and name',
    )
    return { id: text(row.id), file: text(row.file), name: text(row.name) }
  })
}

function classification(value: unknown): Classification {
  requireCondition(
    value === 'DIFFERENT BY DESIGN' ||
      value === 'NO EQUIVALENT' ||
      value === 'NOT A BROWSER BEHAVIOUR',
    'Unknown saved classification',
  )
  return value
}

export function parseSavedStatuses(value: unknown): Map<string, SavedStatus> {
  const entries = Object.entries(object(value)).map(
    ([id, value]): [string, SavedStatus] => {
      const status = object(value).status
      return [
        id,
        status === 'PASS' || status === 'FAIL'
          ? status
          : classification(status),
      ]
    },
  )
  return new Map(entries)
}

function bodyPath(value: unknown): string {
  const path = text(value)
  requireCondition(
    /^scripts\/g(?:1a|1b|1c|2|3)\/[a-z0-9-]+\.js$/.test(path),
    `Invalid input function-body path ${path}`,
  )
  return path.replace(/\.js$/, '.txt')
}

function step(value: unknown): Step {
  if (typeof value === 'string')
    return { kind: 'script', script: bodyPath(value) }
  const row = object(value)
  knownFields(row, ['parallel'])
  const parallel = strings(row.parallel).map(bodyPath)
  const [first, second, third, fourth] = parallel
  requireCondition(
    parallel.length === 4 && first && second && third && fourth,
    'Concurrent capture requires exactly four ordered bodies',
  )
  return { kind: 'parallel', scripts: [first, second, third, fourth] }
}

function steps(row: Record<string, unknown>): readonly [Step, ...Step[]] {
  const normalized =
    row.steps === undefined ? [step(row.script)] : array(row.steps).map(step)
  const [first, ...rest] = normalized
  requireCondition(first, 'Every selected case needs an ordered run step')
  const last = normalized.at(-1)
  requireCondition(
    last?.kind === 'script' && last.script === bodyPath(row.script),
    'Primary body must equal the last ordered body',
  )
  return [first, ...rest]
}

function settings(value: unknown): SettingsPatch | undefined {
  const patch = object(value)
  if (Object.keys(patch).length === 0) return undefined
  if ('tabs' in patch) {
    const tabs = object(patch.tabs)
    knownFields(tabs, ['scope'])
    requireCondition(
      Object.keys(patch).length === 1 && tabs.scope === 'browser',
      'Invalid tab scope patch',
    )
    return { tabs: { scope: 'browser' } }
  }
  const trace = object(patch.trace)
  knownFields(trace, ['valueBytes'])
  requireCondition(
    Object.keys(patch).length === 1 && trace.valueBytes === 262144,
    'Invalid persisted-result retention patch',
  )
  return { trace: { valueBytes: 262144 } }
}

function setup(value: unknown): HostSetup {
  const row = object(value)
  if (row.kind === 'borrowed-page') {
    knownFields(row, ['kind', 'urlTemplate', 'mustCreateOutsideSdk', 'cleanup'])
    requireCondition(
      row.mustCreateOutsideSdk === true && typeof row.cleanup === 'string',
      'Borrowed setup must retain external target ownership and cleanup',
    )
    return { kind: 'borrowed-page', urlTemplate: text(row.urlTemplate) }
  }
  knownFields(row, ['kind', 'deviceScaleFactor'])
  requireCondition(
    row.kind === 'device-scale' && row.deviceScaleFactor === 2,
    'Only native device scale two is in this suite',
  )
  return { kind: 'device-scale', deviceScaleFactor: 2 }
}

function stringMap(value: unknown): Record<string, string> {
  return Object.fromEntries(
    Object.entries(object(value)).map(([key, item]) => [key, text(item)]),
  )
}

function hostCheck(value: unknown): HostCheck {
  const row = object(value)
  switch (row.kind) {
    case 'capture-bytes': {
      knownFields(row, [
        'kind',
        'magicHex',
        'magicUtf8',
        'mimeType',
        'requireImageBlock',
        'requirePersistedExactBytes',
      ])
      const check: HostCheck = { kind: 'capture-bytes' }
      if (row.magicHex !== undefined) check.magicHex = text(row.magicHex)
      if (row.magicUtf8 !== undefined) check.magicUtf8 = text(row.magicUtf8)
      if (row.mimeType !== undefined) check.mimeType = text(row.mimeType)
      if (row.requireImageBlock !== undefined) {
        requireCondition(
          row.requireImageBlock === false,
          'Native captures do not require synthetic image blocks',
        )
        check.requireImageBlock = false
      }
      if (row.requirePersistedExactBytes !== undefined) {
        requireCondition(
          row.requirePersistedExactBytes === true,
          'Persisted captures must compare all bytes',
        )
        check.requirePersistedExactBytes = true
      }
      return check
    }
    case 'download-file':
      knownFields(row, [
        'kind',
        'pathField',
        'contentIncludes',
        'requireExists',
      ])
      requireCondition(
        row.requireExists === true,
        'Download disk evidence is required',
      )
      return {
        kind: 'download-file',
        pathField: text(row.pathField),
        contentIncludes: text(row.contentIncludes),
        requireExists: true,
      }
    case 'download-files':
      knownFields(row, [
        'kind',
        'pathsField',
        'contentIncludes',
        'requireExists',
      ])
      requireCondition(
        row.requireExists === true,
        'Download disk evidence is required',
      )
      return {
        kind: 'download-files',
        pathsField: text(row.pathsField),
        contentIncludes: text(row.contentIncludes),
        requireExists: true,
      }
    case 'typed-json':
      knownFields(row, [
        'kind',
        'count',
        'textRepeat',
        'times',
        'requirePersistedResult',
      ])
      requireCondition(
        row.requirePersistedResult === true,
        'Typed JSON must be compared with persisted results',
      )
      return {
        kind: 'typed-json',
        count: integer(row.count),
        textRepeat: text(row.textRepeat),
        times: integer(row.times),
        requirePersistedResult: true,
      }
    case 'nested-capture-exact-bytes': {
      knownFields(row, [
        'kind',
        'fields',
        'sizes',
        'requiredValue',
        'compareReturnedAndPersistedEveryByte',
        'sameRunId',
      ])
      requireCondition(
        row.compareReturnedAndPersistedEveryByte === true &&
          row.sameRunId === true,
        'Nested captures require same-run byte-for-byte persistence',
      )
      const requiredValue: JsonObject = {}
      for (const [key, value] of Object.entries(object(row.requiredValue)))
        requiredValue[key] = json(value)
      return {
        kind: 'nested-capture-exact-bytes',
        fields: strings(row.fields),
        sizes: stringMap(row.sizes),
        requiredValue,
        compareReturnedAndPersistedEveryByte: true,
        sameRunId: true,
      }
    }
    case 'run-log':
      knownFields(row, ['kind', 'text', 'value'])
      return { kind: 'run-log', text: text(row.text), value: json(row.value) }
    case 'run-error':
      knownFields(row, ['kind', 'messageIncludes', 'requireIsError'])
      requireCondition(
        row.requireIsError === true,
        'The intentional script exception requires the MCP error envelope',
      )
      return {
        kind: 'run-error',
        messageIncludes: text(row.messageIncludes),
        requireIsError: true,
      }
    default:
      throw new Error('Unexpected host check in selected subset')
  }
}

export function parseManifestCases(
  value: unknown,
  originals: ReadonlyMap<string, OriginalCase>,
): ManifestCase[] {
  return array(value).map((value) => {
    const row = object(value)
    const id = text(row.id)
    const original = originals.get(id)
    requireCondition(
      original,
      `Manifest ID has no original inventory row: ${id}`,
    )
    if (row.file !== undefined)
      requireCondition(
        row.file === original.file,
        `Original file differs for ${id}`,
      )
    if (row.name !== undefined)
      requireCondition(
        row.name === original.name,
        `Original title differs for ${id}`,
      )
    if (row.classification !== null)
      return {
        ...original,
        kind: 'classified',
        classification: classification(row.classification),
      }
    requireCondition(
      row.hostContract === undefined &&
        row.expectedFailureChecks === undefined &&
        (row.expectedMismatch === undefined || row.expectedMismatch === null),
      `Excluded mismatch metadata found on selected ${id}`,
    )
    knownFields(row, [
      'id',
      'file',
      'name',
      'proves',
      'script',
      'classification',
      'reason',
      'designEvidence',
      'settings',
      'timeout',
      'steps',
      'postconditions',
      'hostCheck',
      'hostSetup',
      'source',
      'sourceSha',
      'expectedMismatch',
    ])
    const selected: CaseSpec = {
      ...original,
      proves: text(row.proves),
      steps: steps(row),
      timeoutMs: integer(row.timeout),
      expectation: strictFailures.get(id) ?? { kind: 'pass' },
    }
    const patch = settings(row.settings)
    if (patch !== undefined) selected.settings = patch
    if (row.hostSetup !== undefined) selected.hostSetup = setup(row.hostSetup)
    if (row.hostCheck !== undefined)
      selected.hostCheck = hostCheck(row.hostCheck)
    if (row.postconditions !== undefined)
      selected.postconditions = strings(row.postconditions)
    if (row.source !== undefined) {
      const source = text(row.source)
      requireCondition(
        /^packages\/browseros-agent\/contracts\/claw-mcp\/tests\/cases-[a-z-]+\.ts:\d+-\d+$/.test(
          source,
        ),
        'Provenance must name a public repository source location',
      )
      selected.source = source
    }
    if (row.sourceSha !== undefined) {
      const sha = text(row.sourceSha)
      requireCondition(
        /^[a-f0-9]{40}$/.test(sha),
        'Provenance SHA must be a public Git object ID',
      )
      selected.sourceSha = sha
    }
    return { ...original, kind: 'selected', case: selected }
  })
}
