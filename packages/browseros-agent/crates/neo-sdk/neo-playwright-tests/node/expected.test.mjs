/** Neo: The CI gate's verdicts, reruns and generated list, on hand-built Playwright reports. */
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  compare,
  describeChange,
  formatExpected,
  inScopeTests,
  nextExpected,
  PROOF_ATTEMPTS,
  parseExpected,
  passed,
  proofOf,
  refusals,
  settle,
  summaryMarkdown,
  testKey,
} from './expected.mjs'

const scope = {
  schemaVersion: 1,
  reasons: { network: 'network interception' },
  entries: [{ file: 'tests/page/network.spec.ts', reason: 'network' }],
}

/** A JSON report with one spec per [file, title, status] row. */
function report(rows) {
  const results = {
    expected: [{ status: 'passed' }],
    unexpected: [
      {
        status: 'failed',
        error: { message: '\u001b[31mboom\u001b[39m\nstack' },
      },
    ],
    skipped: [],
  }
  return {
    suites: rows.map(([file, title, status]) => ({
      title: file.replace(/^tests\//, ''),
      file: file.replace(/^tests\//, ''),
      specs: [
        {
          title,
          file: file.replace(/^tests\//, ''),
          tests: [{ status, results: results[status] }],
        },
      ],
    })),
  }
}

const run = (rows) => inScopeTests(report(rows), scope)
const list = (rows) =>
  parseExpected(
    formatExpected(
      new Map(
        rows.map(([file, title, outcome]) => [
          testKey(file, title),
          { id: null, file, title, outcome },
        ]),
      ),
      'linux-x64',
    ),
  )
const always = (outcome) => async () => ({ outcome })

test('in-scope tests come back pass, fail or skip; scope.json entries and error colors are dropped', () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'passes', 'expected'],
    ['tests/page/a.spec.ts', 'fails', 'unexpected'],
    ['tests/page/a.spec.ts', 'skips', 'skipped'],
    ['tests/page/network.spec.ts', 'out of scope', 'unexpected'],
  ])
  assert.deepEqual(
    [...tests.values()].map((t) => [t.title, t.outcome, t.error]),
    [
      ['passes', 'pass', undefined],
      ['fails', 'fail', 'boom'],
      ['skips', 'skip', undefined],
    ],
  )
})

test('a run that reproduces the list passes without reruns', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'passes', 'expected'],
    ['tests/page/a.spec.ts', 'fails', 'unexpected'],
    ['tests/page/a.spec.ts', 'skips', 'skipped'],
  ])
  const expected = list([
    ['tests/page/a.spec.ts', 'fails', 'fail'],
    ['tests/page/a.spec.ts', 'skips', 'skip'],
  ])
  const verdict = compare(tests, expected, { complete: true })
  assert.equal(verdict.rerun.length, 0)
  await settle(verdict, always('fail'))
  assert.ok(passed(verdict))
  assert.equal(
    formatExpected(
      nextExpected(tests, expected, verdict, { complete: true }),
      'linux-x64',
    ),
    formatExpected(expected, 'linux-x64'),
  )
})

test('an expected pass that fails is a regression only after failing every rerun alone', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'broken', 'unexpected'],
    ['tests/page/a.spec.ts', 'flaky', 'unexpected'],
  ])
  const verdict = compare(tests, new Map(), { complete: true })
  const calls = []
  await settle(verdict, async (t, attempt) => {
    calls.push([t.title, attempt])
    return { outcome: t.title === 'flaky' && attempt === 2 ? 'pass' : 'fail' }
  })
  assert.deepEqual(calls.sort(), [
    ['broken', 1],
    ['broken', 2],
    ['flaky', 1],
    ['flaky', 2],
  ])
  assert.deepEqual(
    verdict.regressions.map((i) => i.test.title),
    ['broken'],
  )
  assert.deepEqual(
    verdict.flaky.map((i) => i.test.title),
    ['flaky'],
  )
  assert.ok(!passed(verdict))
  const next = nextExpected(tests, new Map(), verdict, { complete: true })
  assert.deepEqual(
    [...next.values()].map((e) => [e.title, e.outcome]),
    [['broken', 'fail']],
  )
})

test('a listed test is newly passing only when it passes every rerun; otherwise it stays listed as flaky', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'fixed', 'expected'],
    ['tests/page/a.spec.ts', 'lucky', 'expected'],
  ])
  const expected = list([
    ['tests/page/a.spec.ts', 'fixed', 'fail'],
    ['tests/page/a.spec.ts', 'lucky', 'fail'],
  ])
  const verdict = compare(tests, expected, { complete: true })
  const calls = []
  await settle(verdict, async (t, attempt) => {
    calls.push([t.title, attempt])
    return { outcome: t.title === 'lucky' ? 'fail' : 'pass' }
  })
  // Leaving the list takes PROOF_ATTEMPTS passing reruns; the lucky one stops at its first failure.
  assert.equal(PROOF_ATTEMPTS, 3)
  assert.deepEqual(calls.sort(), [
    ['fixed', 1],
    ['fixed', 2],
    ['fixed', 3],
    ['lucky', 1],
  ])
  assert.deepEqual(
    verdict.newlyPassing.map((i) => i.test.title),
    ['fixed'],
  )
  assert.deepEqual(
    verdict.flaky.map((i) => i.test.title),
    ['lucky'],
  )
  assert.ok(!passed(verdict))
  const next = nextExpected(tests, expected, verdict, { complete: true })
  assert.deepEqual(
    [...next.values()].map((e) => [e.title, e.outcome]),
    [['lucky', 'fail']],
  )
})

test('a newly skipped expected pass, a fail turned skip and a stale entry all fail without reruns', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'now skipped', 'skipped'],
    ['tests/page/a.spec.ts', 'was failing', 'skipped'],
  ])
  const expected = list([
    ['tests/page/a.spec.ts', 'was failing', 'fail'],
    ['tests/page/gone.spec.ts', 'removed upstream', 'fail'],
  ])
  const verdict = compare(tests, expected, { complete: true })
  let reruns = 0
  await settle(verdict, async () => {
    reruns++
    return { outcome: 'pass' }
  })
  assert.equal(reruns, 0)
  assert.deepEqual(
    verdict.regressions.map((i) => i.test.title),
    ['now skipped'],
  )
  assert.deepEqual(
    verdict.changed.map((i) => i.test.title),
    ['was failing'],
  )
  assert.deepEqual(
    verdict.stale.map((e) => e.title),
    ['removed upstream'],
  )
  const next = nextExpected(tests, expected, verdict, { complete: true })
  assert.deepEqual(
    [...next.values()].map((e) => [e.title, e.outcome]),
    [
      ['now skipped', 'skip'],
      ['was failing', 'skip'],
    ],
  )
})

test('a filtered run judges only its own tests and keeps the entries it did not select', async () => {
  const tests = run([['tests/page/a.spec.ts', 'selected', 'expected']])
  const expected = list([['tests/page/b.spec.ts', 'not selected', 'fail']])
  const verdict = compare(tests, expected, { complete: false })
  await settle(verdict, always('fail'))
  assert.ok(passed(verdict))
  assert.equal(
    nextExpected(tests, expected, verdict, { complete: false }).size,
    1,
  )
})

test('a rerun that broke proves nothing: the test is inconclusive and the run fails', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'expected pass', 'unexpected'],
    ['tests/page/a.spec.ts', 'listed', 'expected'],
    ['tests/page/a.spec.ts', 'listed, fails later', 'expected'],
  ])
  const expected = list([
    ['tests/page/a.spec.ts', 'listed', 'fail'],
    ['tests/page/a.spec.ts', 'listed, fails later', 'fail'],
  ])
  const verdict = compare(tests, expected, { complete: true })
  await settle(verdict, async (t, attempt) => {
    if (t.title === 'expected pass') throw new Error('browser did not start')
    if (t.title === 'listed, fails later')
      return { outcome: attempt === 1 ? 'not-run' : 'fail' }
    return { outcome: attempt === 1 ? 'pass' : 'not-run' }
  })
  assert.deepEqual(
    verdict.inconclusive.map((i) => [
      i.test.title,
      i.reruns.map((r) => r.outcome),
    ]),
    [
      ['expected pass', ['error', 'error']],
      ['listed', ['pass', 'not-run', 'not-run']],
    ],
  )
  // A broken rerun followed by one that fails as listed still confirms the list.
  assert.deepEqual(
    verdict.flaky.map((i) => i.test.title),
    ['listed, fails later'],
  )
  assert.equal(verdict.regressions.length + verdict.newlyPassing.length, 0)
  assert.ok(!passed(verdict))
  const next = nextExpected(tests, expected, verdict, { complete: true })
  assert.deepEqual(
    [...next.values()].map((e) => e.title),
    ['listed', 'listed, fails later'],
  )
})

test('a full-run failure that skips alone settles as the skip it is', async () => {
  // A fixture (here the test server's port) can fail before the spec's own skip runs.
  const tests = run([
    ['tests/page/a.spec.ts', 'listed skip', 'unexpected'],
    ['tests/page/a.spec.ts', 'new skip', 'unexpected'],
  ])
  const expected = list([['tests/page/a.spec.ts', 'listed skip', 'skip']])
  const verdict = compare(tests, expected, { complete: true })
  const calls = []
  await settle(verdict, async (t, attempt) => {
    calls.push([t.title, attempt])
    return { outcome: 'skip' }
  })
  assert.deepEqual(calls.sort(), [
    ['listed skip', 1],
    ['new skip', 1],
    ['new skip', 2],
  ])
  assert.deepEqual(
    verdict.flaky.map((i) => i.test.title),
    ['listed skip'],
  )
  assert.deepEqual(
    verdict.regressions.map((i) => [i.test.title, i.settled]),
    [['new skip', 'skip']],
  )
  const next = nextExpected(tests, expected, verdict, { complete: true })
  assert.deepEqual(
    [...next.values()].map((e) => [e.title, e.outcome]),
    [
      ['listed skip', 'skip'],
      ['new skip', 'skip'],
    ],
  )
})

test('a listed test that passes twice alone and then fails stays listed: two of three is luck, not proof', async () => {
  const tests = run([['tests/page/a.spec.ts', 'mostly', 'expected']])
  const expected = list([['tests/page/a.spec.ts', 'mostly', 'fail']])
  const verdict = compare(tests, expected, { complete: true })
  await settle(verdict, async (_t, attempt) => ({
    outcome: attempt < 3 ? 'pass' : 'fail',
  }))
  assert.deepEqual(
    verdict.flaky.map((i) => i.reruns.map((r) => r.outcome)),
    [['pass', 'pass', 'fail']],
  )
  assert.equal(verdict.newlyPassing.length, 0)
  assert.equal(
    nextExpected(tests, expected, verdict, { complete: true }).size,
    1,
  )
})

test('reruns that disagree, or pass after a failed full run, are a flake: the test stays listed', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'mixed', 'expected'],
    ['tests/page/a.spec.ts', 'skip listed, full run failed', 'unexpected'],
  ])
  const expected = list([
    ['tests/page/a.spec.ts', 'mixed', 'fail'],
    ['tests/page/a.spec.ts', 'skip listed, full run failed', 'skip'],
  ])
  const verdict = compare(tests, expected, { complete: true })
  await settle(verdict, async (t, attempt) => ({
    outcome: t.title === 'mixed' && attempt === 2 ? 'skip' : 'pass',
  }))
  assert.equal(verdict.newlyPassing.length, 0)
  assert.deepEqual(
    verdict.flaky.map((i) => [i.test.title, i.reruns.map((r) => r.outcome)]),
    [
      ['mixed', ['pass', 'skip', 'pass']],
      ['skip listed, full run failed', ['pass', 'pass', 'pass']],
    ],
  )
  const next = nextExpected(tests, expected, verdict, { complete: true })
  assert.equal(next.size, 2)
  assert.deepEqual(refusals(proofOf(tests, expected, next, verdict), next), [])
})

test('a known flaky test is reported with its outcome and never judged, listed or stale', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'flaky, failing', 'unexpected'],
    ['tests/page/a.spec.ts', 'flaky, passing', 'expected'],
  ])
  const entry = (title) => ({
    id: null,
    file: 'tests/page/a.spec.ts',
    title,
    reason: 'flakes',
    tracked: '#1',
  })
  const knownFlaky = new Map(
    ['flaky, failing', 'flaky, passing', 'flaky, gone'].map((title) => [
      testKey('tests/page/a.spec.ts', title),
      entry(title),
    ]),
  )
  // One of them is still on the expected list from before it was known to flake.
  const expected = list([['tests/page/a.spec.ts', 'flaky, passing', 'fail']])
  const verdict = compare(tests, expected, { complete: true, knownFlaky })
  let reruns = 0
  await settle(verdict, async () => {
    reruns++
    return { outcome: 'pass' }
  })
  assert.equal(reruns, 0)
  assert.ok(passed(verdict))
  assert.equal(verdict.stale.length, 0)
  assert.deepEqual(
    verdict.knownFlaky.map((i) => [i.entry.title, i.test?.outcome]),
    [
      ['flaky, failing', 'fail'],
      ['flaky, gone', undefined],
      ['flaky, passing', 'pass'],
    ],
  )
  const next = nextExpected(tests, expected, verdict, {
    complete: true,
    knownFlaky,
  })
  assert.equal(next.size, 0)
  const proof = proofOf(tests, expected, next, verdict, { knownFlaky })
  assert.deepEqual(
    proof.removed.map((r) => r.evidence),
    [{ kind: 'known-flaky' }],
  )
  assert.deepEqual(refusals(proof, next, { knownFlaky }), [])
  // Without the entry in the checkout's known-flaky.tsv, the removal is refused.
  assert.match(
    refusals(proof, next)[0],
    /not on this checkout's known-flaky\.tsv/,
  )
})

test('the update proof: three of three alone after a passing full run, or the test is gone', async () => {
  const tests = run([['tests/page/a.spec.ts', 'fixed', 'expected']])
  const expected = list([
    ['tests/page/a.spec.ts', 'fixed', 'fail'],
    ['tests/page/gone.spec.ts', 'removed upstream', 'fail'],
  ])
  const verdict = compare(tests, expected, { complete: true })
  await settle(verdict, async () => ({ outcome: 'pass' }))
  const next = nextExpected(tests, expected, verdict, { complete: true })
  const proof = proofOf(tests, expected, next, verdict)
  assert.deepEqual(
    proof.removed.map((r) => [r.title, r.evidence]),
    [
      [
        'fixed',
        { kind: 'passed', fullRun: 'pass', reruns: ['pass', 'pass', 'pass'] },
      ],
      ['removed upstream', { kind: 'absent' }],
    ],
  )
  assert.deepEqual(refusals(proof, next), [])
  // A list that drops a test the proof does not cover, or with fewer passing reruns, is refused.
  const short = structuredClone(proof)
  short.removed[0].evidence.reruns = ['pass', 'pass']
  assert.match(
    refusals(short, next)[0],
    /alone pass, pass; leaving the list needs a passing full run and 3 of 3/,
  )
  // A run that reported an error proves no pass, however many reruns passed.
  const broken = proofOf(tests, expected, next, verdict, {
    errors: ['rerun 2 of fixed: browsers survived cleanup'],
  })
  assert.match(
    refusals(broken, next)[0],
    /fixed: the run that passed it reported an error/,
  )
  assert.equal(refusals(broken, next).length, 1)
  const unproven = { ...proof, removed: proof.removed.slice(1) }
  assert.match(
    refusals(unproven, next)[0],
    /fixed: the run gives no proof that it passes/,
  )
})

test('the list is sorted, round-trips, and rejects lines it cannot read', () => {
  const text = formatExpected(
    new Map([
      [
        '2',
        {
          id: 'AB-2',
          outcome: 'skip',
          file: 'tests/page/b.spec.ts',
          title: 'z',
        },
      ],
      [
        '1',
        {
          id: null,
          outcome: 'fail',
          file: 'tests/page/a.spec.ts',
          title: 'y › x',
        },
      ],
    ]),
    'linux-x64',
  )
  const body = text.split('\n').filter((line) => line && !line.startsWith('#'))
  assert.deepEqual(body, [
    '-\tfail\ttests/page/a.spec.ts\ty › x',
    'AB-2\tskip\ttests/page/b.spec.ts\tz',
  ])
  assert.equal(formatExpected(parseExpected(text), 'linux-x64'), text)
  for (const line of [
    'AB-1\tflaky\ttests/page/a.spec.ts\tt',
    'AB-1\tfail\tother/a.ts\tt',
    'AB-1\tfail\ttests/page/a.spec.ts',
  ])
    assert.throws(() => parseExpected(line), /line 1/)
  assert.throws(
    () =>
      parseExpected(
        '-\tfail\ttests/page/a.spec.ts\tt\n-\tskip\ttests/page/a.spec.ts\tt',
      ),
    /twice/,
  )
})

test('the summary names every test the verdict names and how to update the list', async () => {
  const tests = run([
    ['tests/page/a.spec.ts', 'broke <b>|', 'unexpected'],
    ['tests/page/a.spec.ts', 'fixed', 'expected'],
  ])
  const expected = list([['tests/page/a.spec.ts', 'fixed', 'fail']])
  const verdict = compare(tests, expected, { complete: true })
  await settle(verdict, async (t) => ({
    outcome: t.title === 'fixed' ? 'pass' : 'fail',
  }))
  const text = summaryMarkdown({
    verdict,
    committed: expected,
    next: nextExpected(tests, expected, verdict, { complete: true }),
    scores: {
      inScope: { passed: 1, total: 2, skipped: 0, skippedByHarness: 0 },
      all: { passed: 1, total: 2, skipped: 0, skippedByHarness: 0 },
    },
    run: {
      platform: 'linux-x64',
      workers: 16,
      suiteMs: 125_000,
      rerunCount: 4,
      rerunMs: 9_000,
      url: 'https://github.com/o/r/actions/runs/7',
    },
  })
  assert.match(text, /❌ 1 regression, 1 newly passing/)
  assert.match(
    text,
    /broke \\<b\\>\\\|: fail; full run fail, alone: fail, fail — boom/,
  )
  assert.match(
    text,
    /--expected-from https:\/\/github.com\/o\/r\/actions\/runs\/7/,
  )
  assert.match(text, /\| In scope \| 1 \| 2 \| 50\.0% \|/)
  assert.match(text, /Suite 2 min 5 s on 16 workers; 4 reruns alone in 9 s/)
})

test('describeChange says what a new list removes, adds and changes', () => {
  const before = list([
    ['tests/page/a.spec.ts', 'fixed', 'fail'],
    ['tests/page/a.spec.ts', 'kept', 'fail'],
  ])
  const after = list([
    ['tests/page/a.spec.ts', 'kept', 'skip'],
    ['tests/page/a.spec.ts', 'broke', 'fail'],
  ])
  const text = describeChange(before, after)
  assert.match(
    text,
    /^2 → 2 tests listed: 1 removed, 1 added, 1 changed outcome\./,
  )
  assert.match(text, /- - tests\/page\/a.spec.ts › fixed/)
  assert.match(
    text,
    /\+ - tests\/page\/a.spec.ts › broke \(fail; adding a test accepts a regression/,
  )
  assert.match(text, /~ - tests\/page\/a.spec.ts › kept \(fail → skip\)/)
})
