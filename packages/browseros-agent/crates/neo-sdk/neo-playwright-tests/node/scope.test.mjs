/** Score policy must remain independent of failure classification, filters and raw attempts. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { flattenResults } from './report/classify.mjs'
import {
  loadScope,
  recordScope,
  scopeForRun,
  scoreResults,
  scoreText,
  validateScope,
} from './scope.mjs'

function policy(entries) {
  return { ...loadScope(), entries }
}

test('a decision is optional for legacy exclusions, but must name a nonempty decision when present', () => {
  const entry = {
    file: 'tests/page/example.spec.ts',
    ids: ['EX-901'],
    reason: 'decided-deviation',
  }
  assert.doesNotThrow(() => validateScope(policy([entry])))
  assert.doesNotThrow(() =>
    validateScope(
      policy([{ ...entry, decision: 'Owner isolated-world decision' }]),
    ),
  )
  for (const decision of ['', ' ', null, 42])
    assert.throws(
      () => validateScope(policy([{ ...entry, decision }])),
      /Invalid scope entry/,
    )
})

function results(statuses) {
  return {
    suites: [
      {
        title: 'tests/page/example.spec.ts',
        file: 'tests/page/example.spec.ts',
        specs: statuses.map(([id, status, attempts]) => ({
          title: `${id} example`,
          tests: [
            {
              status,
              results: attempts ?? [
                { status: status === 'expected' ? 'passed' : 'failed' },
              ],
            },
          ],
        })),
      },
    ],
  }
}

test('both scores use selected Playwright outcomes; unknown failures stay in scope, skips leave the rate', () => {
  const report = results([
    ['EX-001', 'expected'],
    ['EX-002', 'unexpected'],
    ['EX-003', 'expected'],
    ['EX-004', 'unexpected'],
    ['EX-005', 'skipped', []],
    ['EX-006', 'flaky', [{ status: 'failed' }, { status: 'passed' }]],
    ['EX-007', 'expected', [{ status: 'failed' }]], // An upstream expected failure passes.
    ['EX-008', 'expected', []], // Missing attempts cannot manufacture a pass.
  ])
  const before = structuredClone(report)
  const scores = scoreResults(
    report,
    policy([
      {
        file: 'tests/page/example.spec.ts',
        ids: ['EX-001', 'EX-002'],
        reason: 'network',
      },
      {
        file: 'tests/page/example.spec.ts',
        ids: ['EX-003'],
        reason: 'handles',
      },
      { file: 'tests/page/elsewhere.spec.ts', reason: 'decided-deviation' },
    ]),
  )
  assert.deepEqual(scores.all, {
    passed: 4,
    total: 7,
    skipped: 1,
    skippedByHarness: 0,
  })
  assert.deepEqual(scores.inScope, {
    passed: 2,
    total: 4,
    skipped: 1,
    skippedByHarness: 0,
  })
  assert.equal(
    scoreText(scores.inScope),
    '2/4 (50.0%); 1 skipped, not counted (0 by the harness)',
  )
  assert.deepEqual(scores.byReason.network, { total: 2, passed: 1 })
  assert.deepEqual(scores.byReason.handles, { total: 1, passed: 1 })
  assert.deepEqual(scores.byReason['decided-deviation'], {
    total: 0,
    passed: 0,
  })
  assert.deepEqual(
    scores.passingExcluded.map((row) => row.id),
    ['EX-001', 'EX-003'],
  )
  assert.deepEqual(report, before)
})

test('file selectors count each selected case once and empty selections have no passes', () => {
  const manifest = policy([
    { file: 'tests/page/example.spec.ts', reason: 'network' },
  ])
  assert.deepEqual(
    scoreResults(results([['EX-001', 'expected']]), manifest).inScope,
    { passed: 0, total: 0, skipped: 0, skippedByHarness: 0 },
  )
  assert.deepEqual(scoreResults({ suites: [] }, manifest).all, {
    passed: 0,
    total: 0,
    skipped: 0,
    skippedByHarness: 0,
  })
})

test('repeat-each instances count independently while offline joins still require unique cases', () => {
  const report = results([
    ['EX-001', 'expected'],
    ['EX-002', 'unexpected'],
  ])
  report.suites[0].specs[0].tests.push({
    status: 'unexpected',
    results: [{ status: 'failed' }],
  })
  report.suites[0].specs[1].tests.push({
    status: 'flaky',
    results: [{ status: 'passed' }],
  })
  const scores = scoreResults(
    report,
    policy([
      {
        file: 'tests/page/example.spec.ts',
        ids: ['EX-001'],
        reason: 'network',
      },
    ]),
  )
  const none = { skipped: 0, skippedByHarness: 0 }
  assert.deepEqual(scores.all, { passed: 2, total: 4, ...none })
  assert.deepEqual(scores.inScope, { passed: 1, total: 2, ...none })
  assert.deepEqual(scores.byReason.network, { passed: 1, total: 2 })
  assert.equal(scores.passingExcluded.length, 1)
  assert.throws(() => flattenResults(report), /Duplicate case identity/)
})

test('skips are split by who made them: the harness, or the spec itself', () => {
  const report = results([
    ['EX-001', 'expected'],
    ['EX-002', 'skipped', []],
    ['EX-003', 'skipped', []],
    ['EX-004', 'skipped', []],
    ['EX-005', 'skipped', []],
  ])
  const [, loop, internals, spec, fixme] = report.suites[0].specs
  loop.tests[0].annotations = [
    { type: 'expectLoop', description: 'toBe' },
    { type: 'skip', description: 'expectLoop: toBe is not covered' },
  ]
  internals.tests[0].annotations = [
    { type: 'skip', description: 'needs Playwright internals: toImpl' },
  ]
  spec.tests[0].annotations = [
    { type: 'skip', description: 'needs workerScriptLoaded event' },
  ]
  fixme.tests[0].annotations = [{ type: 'fixme' }]
  const scores = scoreResults(report, policy([]))
  assert.deepEqual(scores.inScope, {
    passed: 1,
    total: 1,
    skipped: 4,
    skippedByHarness: 2,
  })
  assert.deepEqual(scores.all, scores.inScope)
})

test('policy rejects unknown reasons, malformed IDs and overlapping selectors', () => {
  const entry = {
    file: 'tests/page/example.spec.ts',
    ids: ['EX-001'],
    reason: 'network',
  }
  for (const entries of [
    [{ ...entry, reason: 'failing' }],
    [{ ...entry, file: '../example.spec.ts' }],
    [{ ...entry, ids: [] }],
    [{ ...entry, ids: ['EX-001', 'EX-001'] }],
    [{ ...entry, ids: ['not-an-id'] }],
    [entry, { ...entry, reason: 'handles' }],
    [entry, { ...entry, ids: undefined }],
    [{ ...entry, ids: undefined }, entry],
  ])
    assert.throws(() => validateScope(policy(entries)), /scope/i)
  assert.throws(() => validateScope(null), /scope/i)
})

test('reasons come from the manifest, so a recorded run keeps one the policy has retired', () => {
  const current = loadScope()
  assert.equal(current.reasons.drift, undefined)
  const recorded = {
    ...current,
    reasons: { ...current.reasons, drift: 'Newer than the engine.' },
    entries: [
      { file: 'tests/page/example.spec.ts', ids: ['EX-001'], reason: 'drift' },
    ],
  }
  const scores = scoreResults(
    results([
      ['EX-001', 'unexpected'],
      ['EX-002', 'expected'],
    ]),
    recorded,
  )
  assert.deepEqual(scores.byReason.drift, { total: 1, passed: 0 })
  assert.deepEqual(scores.inScope, {
    passed: 1,
    total: 1,
    skipped: 0,
    skippedByHarness: 0,
  })
  for (const reasons of [{}, { ...current.reasons, drift: ' ' }])
    assert.throws(() => validateScope({ ...recorded, reasons }), /scope/i)
})

test('historical reports use the recorded manifest and reject tampering', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-scope-'))
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
  const scope = recordScope(directory)
  const run = { directory, metadata: { scope } }
  assert.deepEqual(scopeForRun(run), loadScope())
  fs.appendFileSync(path.join(directory, 'scope.json'), '\n')
  assert.throws(() => scopeForRun(run), /hash differs/)
  assert.deepEqual(scopeForRun({ metadata: {} }), loadScope())
})
