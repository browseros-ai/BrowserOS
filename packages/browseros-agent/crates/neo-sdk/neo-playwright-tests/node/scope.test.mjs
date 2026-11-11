/** Score policy must remain independent of failure classification, filters and raw attempts. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import {
  loadScope,
  recordScope,
  scopeForRun,
  scoreResults,
  validateScope,
} from './scope.mjs'

function policy(entries) {
  return { ...loadScope(), entries }
}

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

test('both scores use selected Playwright outcomes; unknown failures and skips stay in scope', () => {
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
      { file: 'tests/page/elsewhere.spec.ts', reason: 'drift' },
    ]),
  )
  assert.deepEqual(scores.all, { passed: 4, total: 8 })
  assert.deepEqual(scores.inScope, { passed: 2, total: 5 })
  assert.deepEqual(scores.byReason.network, { total: 2, passed: 1 })
  assert.deepEqual(scores.byReason.handles, { total: 1, passed: 1 })
  assert.deepEqual(scores.byReason.drift, { total: 0, passed: 0 })
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
    { passed: 0, total: 0 },
  )
  assert.deepEqual(scoreResults({ suites: [] }, manifest).all, {
    passed: 0,
    total: 0,
  })
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
