import { createHash } from 'node:crypto'
import type { CaseSpec, Expectation, Step } from '../model'

/** The first suite has a fixed membership boundary; future differences require their own scope. */
export const expectedFamilyCounts: Readonly<Record<string, number>> = {
  'cases-act.ts': 17,
  'cases-capture-io.ts': 23,
  'cases-click-recovery.ts': 24,
  'cases-fill.ts': 33,
  'cases-frame-refs.ts': 11,
  'cases-isolation.ts': 2,
  'cases-labels.ts': 1,
  'cases-navigate-snapshot.ts': 12,
  'cases-outputs.ts': 2,
  'cases-read-eval.ts': 13,
  'cases-screenshot-scale.ts': 1,
  'cases-sdk-input.ts': 39,
  'cases-sdk-navigation.ts': 16,
  'cases-sdk-read-eval.ts': 4,
  'cases-snapshot-concurrency.ts': 6,
  'cases-tabs.ts': 8,
}

export const selectedIdDigest =
  '13ff64f23fa82e93ec75abfd44e4d66e7038ac7ea6ffadc5d0d5a0faa2c09cfa'

export const strictFailures: ReadonlyMap<string, Expectation> = new Map([
  [
    'labels-001',
    {
      kind: 'xfail',
      reason:
        "The snapshot omits the clickable ref for the hidden checkbox's visible label.",
      signature: 'hidden-label-ref',
    },
  ],
  [
    'isolation-001',
    {
      kind: 'xfail',
      reason:
        'Compact snapshot output omits the named child control inside a clickable card.',
      signature: 'isolated-child-ref',
    },
  ],
  [
    'isolation-002',
    {
      kind: 'xfail',
      reason:
        'Compact snapshot output omits the named child control inside a clickable card.',
      signature: 'card-child-ref',
    },
  ],
  [
    'sdk-input-022',
    {
      kind: 'xfail',
      reason:
        'Rejected invalid date fill clears the previous date without restoring it.',
      signature: 'invalid-date-reset',
    },
  ],
  [
    'sdk-input-045',
    {
      kind: 'xfail',
      reason:
        'Rejected invalid date fill clears the previous date without restoring it.',
      signature: 'invalid-date-reset',
    },
  ],
])

export function requireCondition(
  condition: unknown,
  message: string,
): asserts condition {
  if (!condition) throw new Error(message)
}

export function sha256(bytes: Uint8Array | string): string {
  return createHash('sha256').update(bytes).digest('hex')
}

export function bodyPaths(step: Step): readonly string[] {
  return step.kind === 'script' ? [step.script] : step.scripts
}

export function requireBodyPath(path: string): void {
  requireCondition(
    /^scripts\/g(?:1a|1b|1c|2|3)\/[a-z0-9-]+\.txt$/.test(path),
    `Invalid function-body path ${path}`,
  )
}

function countBy(values: readonly string[]): Record<string, number> {
  const counts: Record<string, number> = {}
  for (const value of values) counts[value] = (counts[value] ?? 0) + 1
  return counts
}

function requireCounts(
  actual: Record<string, number>,
  expected: Readonly<Record<string, number>>,
) {
  requireCondition(
    JSON.stringify(Object.entries(actual).sort()) ===
      JSON.stringify(Object.entries(expected).sort()),
    `Inventory counts differ: ${JSON.stringify(actual)}`,
  )
}

/** Checks semantic coverage independently of serialization and body hashes. */
export function validateCaseContract(cases: readonly CaseSpec[]): void {
  requireCondition(
    cases.length === 212,
    'The first suite must contain 212 expanded cases',
  )
  const ids = cases.map((entry) => entry.id)
  requireCondition(
    new Set(ids).size === 212,
    'Expanded case IDs must be unique',
  )
  requireCondition(
    sha256(`${[...ids].sort().join('\n')}\n`) === selectedIdDigest,
    'Selected case ID digest differs from the approved first suite',
  )
  requireCondition(
    sha256(
      `${cases.map(({ file, id, name }) => JSON.stringify([file, id, name])).join('\n')}\n`,
    ) === '980fcaf5fff6c013611566988d8c7bfadf0f5015d3006f82c3137eaf36685efb',
    'Historical file, ID and original title identities differ',
  )
  requireCounts(countBy(cases.map((entry) => entry.file)), expectedFamilyCounts)
  requireCounts(countBy(cases.map((entry) => String(entry.timeoutMs))), {
    '20000': 189,
    '30000': 8,
    '40000': 4,
    '60000': 10,
    '90000': 1,
  })
  requireCondition(
    JSON.stringify(ids) ===
      JSON.stringify(
        [...cases]
          .sort(
            (a, b) => a.file.localeCompare(b.file) || a.id.localeCompare(b.id),
          )
          .map((entry) => entry.id),
      ),
    'Registry order must be original filename followed by expanded ID',
  )
  const primary = new Set<string>()
  const bodies = new Set<string>()
  let parallelGroups = 0
  for (const entry of cases) {
    requireCondition(
      entry.file === `cases-${entry.id.replace(/-\d{3}$/, '')}.ts`,
      `ID/file mismatch for ${entry.id}`,
    )
    requireCondition(
      entry.name.length > 0 && entry.proves.length > 0,
      `Missing title or intent for ${entry.id}`,
    )
    requireCondition(
      JSON.stringify(entry.expectation) ===
        JSON.stringify(strictFailures.get(entry.id) ?? { kind: 'pass' }),
      `Unexpected failure contract for ${entry.id}`,
    )
    const last = entry.steps.at(-1)
    if (last?.kind !== 'script')
      throw new Error(`Final step must be a sequential script for ${entry.id}`)
    primary.add(last.script)
    for (const step of entry.steps) {
      if (step.kind === 'parallel') {
        parallelGroups += 1
        requireCondition(
          step.scripts.length === 4,
          `Parallel capture requires four requests for ${entry.id}`,
        )
        requireCondition(
          new Set(step.scripts).size === 1,
          `Concurrent captures must use the same body for ${entry.id}`,
        )
      }
      for (const path of bodyPaths(step)) {
        requireBodyPath(path)
        bodies.add(path)
      }
    }
  }
  requireCondition(
    primary.size === 150,
    'Expected 150 primary function-body paths',
  )
  requireCondition(
    bodies.size === 155,
    'Expected 155 distinct function-body paths',
  )
  requireCondition(
    parallelGroups === 2,
    'Expected two four-request concurrent groups',
  )
  const withField = (field: 'settings' | 'hostCheck' | 'hostSetup') =>
    cases.filter((entry) => entry[field] !== undefined).map((entry) => entry.id)
  const requireIds = (actual: string[], expected: string[]) =>
    requireCondition(
      JSON.stringify(actual.sort()) === JSON.stringify(expected.sort()),
      `Case metadata membership differs: ${actual.join(', ')}`,
    )
  requireIds(
    cases.filter((entry) => entry.steps.length > 1).map((entry) => entry.id),
    [
      'navigate-snapshot-015',
      'snapshot-concurrency-007',
      'snapshot-concurrency-008',
    ],
  )
  requireIds(withField('settings'), [
    'capture-io-012',
    'capture-io-018',
    'capture-io-019',
    'outputs-007',
    'outputs-009',
  ])
  requireCondition(
    cases.filter((entry) => entry.settings && 'trace' in entry.settings)
      .length === 4,
    'Expected four persisted-result retention patches',
  )
  requireIds(withField('hostSetup'), ['capture-io-012', 'screenshot-scale-002'])
  requireIds(withField('hostCheck'), [
    'capture-io-001',
    'capture-io-002',
    'capture-io-003',
    'capture-io-017',
    'capture-io-018',
    'capture-io-019',
    'capture-io-027',
    'outputs-007',
    'outputs-009',
    'read-eval-020',
    'read-eval-021',
  ])
  requireCondition(
    cases.filter((entry) => entry.expectation.kind === 'pass').length ===
      cases.length - strictFailures.size,
    'Live passing contracts must follow the current strict failure policy',
  )
}
