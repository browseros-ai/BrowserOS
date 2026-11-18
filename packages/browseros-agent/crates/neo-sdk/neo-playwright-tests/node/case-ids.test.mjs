/** Pinned case IDs must cover exactly the collected corpus and never fall back silently. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { corpus } from '../js/prepare.mjs'
import { loadCaseIds, pinnedCaseId, verifyCaseIds } from './case-ids.mjs'
import { caseLabel, flattenResults } from './report/classify.mjs'

const fill = 'tests/page/page-fill.spec.ts'

test('the committed map is for the pinned corpus and names only included suites', () => {
  const map = loadCaseIds()
  assert.equal(map.pin, corpus.revision)
  const included = new Set(
    corpus.suites.filter((suite) => suite.include).map((suite) => suite.file),
  )
  for (const key of map.ids.keys())
    assert.ok(included.has(JSON.parse(key)[0]), key)
  // Same file and title as upstream's later registry: the ID is unchanged.
  assert.equal(pinnedCaseId(fill, 'should fill range input'), 'FI-005')
  // Not in upstream's registry: a 9xx number of the file's own code.
  assert.equal(
    pinnedCaseId(fill, 'should throw on incorrect range value'),
    'FI-901',
  )
})

test('verification rejects a missing case, a stale entry and another pin', () => {
  const map = {
    pin: corpus.revision,
    ids: new Map([[JSON.stringify([fill, 'kept']), 'FI-001']]),
  }
  const kept = { suite: fill, title: 'kept' }
  verifyCaseIds([kept], corpus.revision, map)
  assert.throws(
    () =>
      verifyCaseIds(
        [kept, { suite: fill, title: 'new' }],
        corpus.revision,
        map,
      ),
    /no case ID/,
  )
  assert.throws(() => verifyCaseIds([], corpus.revision, map), /names no/)
  assert.throws(() => verifyCaseIds([kept], 'other', map), /pin differs/)
})

test('malformed maps are rejected', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'case-ids-'))
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
  const file = path.join(directory, 'case-ids.json')
  for (const manifest of [
    { schemaVersion: 2, pin: 'x', files: {} },
    { schemaVersion: 1, files: {} },
    { schemaVersion: 1, pin: 'x', files: { [fill]: { title: 'fi-1' } } },
  ]) {
    fs.writeFileSync(file, JSON.stringify(manifest))
    assert.throws(() => loadCaseIds(file), /case ID/)
  }
})

test('results take the pinned ID; results from earlier pins keep the ID in their title', () => {
  const report = {
    suites: [
      {
        title: 'page/page-fill.spec.ts',
        file: 'page/page-fill.spec.ts',
        specs: [
          { title: 'should fill range input', tests: [] },
          { title: 'FI-034 should retry on inert element', tests: [] },
        ],
      },
    ],
  }
  const rows = flattenResults(report, corpus.suites)
  assert.deepEqual(
    rows.map((row) => row.caseId),
    ['FI-005', 'FI-034'],
  )
  assert.equal(
    caseLabel(rows[0].caseId, rows[0].title),
    'FI-005 should fill range input',
  )
  assert.equal(
    caseLabel(rows[1].caseId, rows[1].title),
    'FI-034 should retry on inert element',
  )
  assert.equal(
    caseLabel(
      'DD-006',
      'Drag and drop › iframe › DD-006 should drag into an iframe',
    ),
    'Drag and drop › iframe › DD-006 should drag into an iframe',
  )
})
