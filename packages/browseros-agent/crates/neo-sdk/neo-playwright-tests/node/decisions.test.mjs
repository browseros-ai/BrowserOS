/** Decision exclusions are negative tests, independent of the positive score and expected list. */
import assert from 'node:assert/strict'
import test from 'node:test'
import { decisionErrors, judgeDecisions } from './decisions.mjs'

const file = 'tests/page/example.spec.ts'
const decision = 'D-isolated: engine reads stay in the isolated world'
const scope = {
  schemaVersion: 1,
  reasons: {
    'decided-deviation': 'isolated-world engine',
    network: 'no Network',
  },
  entries: [
    { file, ids: ['EX-901'], reason: 'decided-deviation', decision },
    { file, ids: ['EX-902'], reason: 'network' },
  ],
}
const report = (statuses) => ({
  suites: [
    {
      title: file,
      file,
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
})
const judge = (rows, complete = true) =>
  judgeDecisions(report(rows), scope, { complete })

test('only entries carrying a decision are judged; legacy excluded passes remain tolerated', () => {
  const results = report([
    ['EX-901', 'unexpected'],
    ['EX-902', 'expected'],
  ])
  const original = structuredClone(results)
  const verdict = judgeDecisions(results, scope, { complete: true })
  assert.deepEqual(verdict, { total: 1, failed: 1, violations: [] })
  assert.deepEqual(decisionErrors(verdict), [])
  assert.deepEqual(results, original)
})

test('a passing negative test names its ID and decision; no expected-list or flaky waiver applies', () => {
  const verdict = judge([['EX-901', 'expected']])
  assert.equal(verdict.failed, 0)
  assert.match(
    decisionErrors(verdict)[0],
    /EX-901.*passed although decision "D-isolated/,
  )
  assert.match(
    decisionErrors(verdict)[0],
    /the design changed, or this entry is wrong/,
  )
})

test('skips, empty attempts and interrupted runs do not prove a negative test still fails', () => {
  for (const [status, attempts, outcome] of [
    ['skipped', [], 'skip'],
    ['unexpected', [], 'harness-error'],
    ['unexpected', [{ status: 'interrupted' }], 'harness-error'],
  ]) {
    const verdict = judge([['EX-901', status, attempts]])
    assert.equal(verdict.failed, 0)
    assert.equal(verdict.violations[0].outcome, outcome)
    assert.match(
      decisionErrors(verdict)[0],
      /a negative test must run and fail/,
    )
  }
})

test('a successful attempt is a violation even when the overall status says unexpected or flaky', () => {
  for (const status of ['unexpected', 'flaky']) {
    const verdict = judge([
      ['EX-901', status, [{ status: 'passed' }, { status: 'failed' }]],
    ])
    assert.equal(verdict.violations[0].outcome, 'pass')
    assert.match(decisionErrors(verdict)[0], /passed although decision/)
  }
})

test('complete runs require every decision selector; filtered runs judge only what was selected', () => {
  assert.deepEqual(judge([], false), { total: 0, failed: 0, violations: [] })
  const verdict = judge([])
  assert.equal(verdict.total, 1)
  assert.equal(verdict.violations[0].id, 'EX-901')
  assert.equal(verdict.violations[0].outcome, 'missing')
})

test('whole-file decisions judge every case and cannot disappear from a complete run', () => {
  const policy = {
    ...scope,
    entries: [{ file, reason: 'decided-deviation', decision }],
  }
  const verdict = judgeDecisions(
    report([
      ['EX-901', 'unexpected'],
      ['EX-902', 'unexpected'],
    ]),
    policy,
    { complete: true },
  )
  assert.deepEqual(verdict, { total: 2, failed: 2, violations: [] })
  assert.equal(
    judgeDecisions(report([]), policy, { complete: true }).violations[0]
      .outcome,
    'missing',
  )
})
