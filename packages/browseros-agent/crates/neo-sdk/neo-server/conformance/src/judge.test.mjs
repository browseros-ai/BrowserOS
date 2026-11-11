import { expect, test } from 'bun:test'
import pairs from './fixtures/review-diagnostic-pairs.json'
import { classify, judge } from './judge.mjs'

const row = { id: 'x', selected: true, class: 'runnable' }
const record = (side, extra = {}) => ({
  id: 'x',
  attempt: 1,
  status: 'passed',
  expectedStatus: 'passed',
  assertions: { called: 1 },
  errors: [],
  provenance: {
    side,
    programSha256: 'same',
    browserVersion: 'Chrome/155',
    playwrightCore: '1.64.1',
    neo: {
      runId: 'run',
      binarySha256: 'hash',
      gitSha: 'sha',
      opCount: 1,
      traceVerified: true,
    },
  },
  ...extra,
})
const check = (p, n, opts) => classify(row, p, n, opts)
test('missing, transport, hashes and trace failures never count as behavior', () => {
  expect(check(null, record('neo')).classification).toBe('harness-error')
  expect(
    check(
      record('native'),
      record('neo', { transport: { error: 'closed port' } }),
    ).classification,
  ).toBe('harness-error')
  const n = record('neo')
  n.provenance.programSha256 = 'other'
  expect(check(record('native'), n).classification).toBe('harness-error')
  n.provenance.neo.traceVerified = false
  expect(check(record('native'), n).classification).toBe('harness-error')
})
test('symmetric harness exclusions, environment and skips precede verdicts', () => {
  const errors = [{ harnessUnavailable: 'server.setRoute' }]
  expect(
    check(
      record('native', { status: 'failed', errors }),
      record('neo', { status: 'failed', errors }),
    ).classification,
  ).toBe('harness-unavailable')
  expect(
    check(record('native'), record('neo', { errors })).classification,
  ).toBe('harness-error')
  expect(
    check(
      record('native'),
      record('neo', { errors: [{ owner: 'neo', code: 'cdp_timeout' }] }),
    ).classification,
  ).toBe('harness-error')
  const skipped = {
    status: 'skipped',
    expectedStatus: 'skipped',
    skip: { reason: 'same' },
  }
  expect(
    check(record('native', skipped), record('neo', skipped)).classification,
  ).toBe('skipped')
})
test('oracle owns native failures, expected failures are compared correctly', () => {
  expect(
    check(
      record('native', { status: 'failed' }),
      record('neo', { status: 'failed' }),
    ).classification,
  ).toBe('oracle-fail')
  expect(
    check(
      record('native', { status: 'failed', expectedStatus: 'failed' }),
      record('neo', { status: 'failed', expectedStatus: 'failed' }),
    ).classification,
  ).toBe('pass')
})
test('exact deviations, traced refusals, missing globals and assertion counts', () => {
  const n = record('neo', {
    status: 'failed',
    errors: [{ code: 'gone', message: 'gone' }],
  })
  expect(
    check(record('native'), n, {
      deviations: [
        {
          test: 'x',
          neo: { status: 'failed', code: 'gone', message: '^gone$' },
          reason: 'strict refs',
          decision: 'owner',
        },
      ],
    }).classification,
  ).toBe('deviation')
  n.refusals = [
    { code: 'method_unavailable', owner: 'script', api: 'page.route' },
  ]
  expect(
    check(record('native'), n, { unsupported: ['page.route'] }).classification,
  ).toBe('fail')
  n.errors = n.refusals
  expect(
    check(record('native'), n, { unsupported: ['page.route'] }),
  ).toMatchObject({ classification: 'unsupported', gating: false })
  n.refusals = []
  n.errors = [{ name: 'ReferenceError', message: 'atob is not defined' }]
  expect(check(record('native'), n).gating).toBe(true)
  expect(
    check(record('native'), record('neo', { assertions: { called: 2 } }))
      .classification,
  ).toBe('fail:assertion-count')
})
test('stale entries fail the gate', () => {
  expect(
    judge({
      inventory: { tests: [row] },
      native: [record('native')],
      neo: [record('neo')],
      deviations: [{ test: 'x' }],
    }).ok,
  ).toBe(false)
})
test('an asymmetric skip or expected failure cannot pass without a valid oracle', () => {
  expect(
    check(
      record('native', { status: 'skipped', expectedStatus: 'skipped' }),
      record('neo', { assertions: { called: 0 } }),
    ).gating,
  ).toBe(true)
  expect(
    check(
      record('native'),
      record('neo', { status: 'failed', expectedStatus: 'failed' }),
    ).gating,
  ).toBe(true)
})
test('repeats retain an earlier failure and mark changing verdicts flaky', () => {
  const p = record('native'),
    n = record('neo')
  const r = judge({
    inventory: { tests: [row] },
    repeat: 2,
    native: [p, { ...p, attempt: 2 }],
    neo: [record('neo', { status: 'failed' }), { ...n, attempt: 2 }],
  })
  expect(r.ok).toBe(false)
  expect(r.results.length).toBe(2)
  expect(r.results.every((r) => r.flaky)).toBe(true)
})

test('a caught refusal and an incompatible error cannot become diagnostic passes', () => {
  const site = {
    line: 267,
    column: 2,
    matcher: 'toContain',
    kind: 'message',
    rejects: false,
  }
  const p = record('native', {
    diagnostics: [
      {
        ...site,
        matched: true,
        error: {
          name: 'Error',
          message:
            'expect(locator).toHaveText(expected) failed\nError: Frame locators are not allowed inside composite locators',
        },
      },
    ],
  })
  const n = record('neo', {
    diagnostics: [
      {
        ...site,
        matched: false,
        error: {
          name: 'Error',
          code: 'no_match',
          message:
            'expect(locator).toHaveText(expected) failed\nError: element(s) not found',
        },
      },
    ],
  })
  expect(check(p, n)).toMatchObject({ classification: 'fail', gating: true })
  n.refusals = [
    {
      owner: 'script',
      code: 'method_unavailable',
      api: 'locator.waitForFunction',
    },
  ]
  expect(
    check(p, n, { unsupported: ['locator.waitForFunction'] }),
  ).toMatchObject({ classification: 'fail', gating: true })
})
test('wording can differ after the same diagnostic failure path', () => {
  const diag = (message, matched) => ({
    line: 4,
    column: 1,
    matcher: 'toThrow',
    kind: 'throw',
    rejects: false,
    error: { name: 'TypeError', message },
    matched,
  })
  expect(
    check(
      record('native', { diagnostics: [diag('expected', true)] }),
      record('neo', { diagnostics: [diag('different', false)] }),
    ).classification,
  ).toBe('pass')
})
test('attempts are paired by exact ordinal and duplicates are rejected', () => {
  const p = record('native'),
    n = record('neo')
  const report = judge({
    inventory: { tests: [row] },
    repeat: 2,
    native: [p],
    neo: [{ ...n, attempt: 2 }],
  })
  expect(report.ok).toBe(false)
  expect(report.results.map((result) => result.classification)).toEqual([
    'harness-error',
    'harness-error',
  ])
  expect(
    judge({ inventory: { tests: [row] }, native: [p, p], neo: [n] }).runErrors,
  ).toEqual(['native: duplicate attempt x #1'])
})

test('JSON key order, auxiliary code and call logs leave the same diagnostic path equivalent', () => {
  const native = {
    line: 4,
    column: 1,
    matcher: 'toContain',
    kind: 'message',
    rejects: false,
    matched: true,
    error: {
      name: 'Error',
      message: 'page.fill: Error: Malformed value\nCall log: native',
    },
  }
  const neo = {
    error: {
      code: 'invalid_input',
      message: 'page.fill: Error: Malformed value\nCall log: neo',
      name: 'Error',
    },
    matched: true,
    rejects: false,
    kind: 'message',
    matcher: 'toContain',
    column: 1,
    line: 4,
  }
  expect(
    check(
      record('native', { diagnostics: [native] }),
      record('neo', { diagnostics: [neo] }),
    ).classification,
  ).toBe('pass')
})

// These are the retained diagnostics from the Linux review. Runner provenance fields are
// supplied by the fixture builder so the test isolates error and refusal classification.
for (const pair of pairs) {
  test(`retained diagnostic pair: ${pair.id}`, () => {
    const p = record('native', pair.native)
    const n = record('neo', pair.neo)
    const unsupported = (n.refusals || []).map(({ api }) => api)
    expect(check(p, n, { unsupported })).toMatchObject({
      classification: pair.expected,
      gating: pair.expected === 'fail',
    })
  })
}

test('an earlier caught diagnostic refusal cannot erase a later mismatch or terminal failure', () => {
  const caught = pairs.find(({ expected }) => expected === 'unsupported')
  const nested = pairs.find(({ id }) =>
    id.endsWith('should allow some, but not all nested frameLocators'),
  )
  const combined = (side) =>
    record(side, {
      assertions: {
        called: caught[side].assertions.called + nested[side].assertions.called,
      },
      diagnostics: [...caught[side].diagnostics, ...nested[side].diagnostics],
      refusals: caught[side].refusals,
    })
  const p = combined('native'),
    n = combined('neo')
  const unsupported = n.refusals.map(({ api }) => api)
  expect(check(p, n, { unsupported })).toMatchObject({
    classification: 'fail',
    gating: true,
    reason: expect.stringContaining('composite-frame-validation vs no_match'),
  })
  expect(
    check(
      record('native', caught.native),
      record('neo', {
        ...caught.neo,
        status: 'failed',
        errors: [nested.neo.diagnostics[0].error],
      }),
      { unsupported },
    ),
  ).toMatchObject({ classification: 'fail', gating: true })
})
