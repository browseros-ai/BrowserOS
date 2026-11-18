/** Neo: Small Playwright JSON fixtures exercise public verdicts, evidence, and bridge identities. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import test from 'node:test'
import { expectLoop } from '../expect-loop.mjs'
import {
  caseKey,
  classifyRuns,
  compareBridge,
  countVerdicts,
  flattenResults,
} from './classify.mjs'
import { refusals } from './refusals.mjs'

function fixture(name) {
  return JSON.parse(
    fs.readFileSync(
      new URL(`./fixtures/${name}.json`, import.meta.url),
      'utf8',
    ),
  )
}

function report(cases, errors = []) {
  return {
    suites: [
      {
        title: 'page/example.spec.ts',
        file: 'page/example.spec.ts',
        specs: cases.map((entry, index) => ({
          title: `EX-${String(index).padStart(3, '0')} example`,
          file: 'page/example.spec.ts',
          line: index + 1,
          column: 1,
          tests: [
            {
              expectedStatus: 'passed',
              status: 'unexpected',
              results: [{ status: 'failed', errors: [{ message: entry }] }],
              ...(typeof entry === 'object' ? entry : {}),
            },
          ],
        })),
      },
    ],
    errors,
  }
}

function classify(errors, referenceErrors = []) {
  const neo = report(errors)
  const reference = report(
    errors.map(
      (_, index) =>
        referenceErrors[index] || {
          status: 'expected',
          results: [{ status: 'passed' }],
        },
    ),
  )
  return classifyRuns(neo, reference)
}

test('tests that loop diagnostics cannot misclassify assertion data or lose a missing read', async () => {
  class Locator {}
  const missing = await expectLoop(new Locator())
    .toHaveJSProperty('value', {
      first:
        'this expected object is intentionally long enough to wrap across multiple lines',
      second: 'so the missing read must remain visible in the first error line',
    })
    .catch((error) => error)
  const mismatch = await expectLoop({ textContent: async () => 'actual' })
    .toHaveText('worker process exited unexpectedly', { timeout: 5 })
    .catch((error) => error)
  assert.deepEqual(
    classify([missing.message, mismatch.message]).map((row) => row.verdict),
    ['neo-lacks', 'neo-differs'],
  )
})

test('tests that annotated passes are separate and skip reasons survive', () => {
  const rows = classify([
    {
      status: 'expected',
      annotations: [{ type: 'expectLoop', description: 'toBeVisible' }],
      results: [{ status: 'passed' }],
    },
    { status: 'expected', results: [{ status: 'passed' }] },
    {
      status: 'skipped',
      annotations: [
        {
          type: 'skip',
          description: 'expectLoop: toMatchAriaSnapshot is not covered',
        },
      ],
      results: [{ status: 'skipped' }],
    },
    {
      status: 'expected',
      results: [
        {
          status: 'passed',
          annotations: [{ type: 'expectLoop', description: 'toHaveText' }],
        },
      ],
    },
  ])
  assert.deepEqual(
    rows.map((row) => row.verdict),
    ['pass via expectLoop', 'pass', 'skip', 'pass via expectLoop'],
  )
  assert.deepEqual(rows[2].skipReasons, [
    'expectLoop: toMatchAriaSnapshot is not covered',
  ])
  assert.equal(rows[2].reason, 'expectLoop: toMatchAriaSnapshot is not covered')
  assert.equal(countVerdicts(rows)['pass via expectLoop'], 2)
})

test('tests that expectLoop failures classify public reads without scanning expected text as internals', () => {
  const rows = classify([
    'expectLoop: toHaveText expected "_expect is not a function"; last received "no"',
    'AssertionError: expectLoop: toHaveValue expected "x"; last received undefined; read failed: Locator.inputValue is not a function',
    'expectLoop: toHaveText expected "x"; last received "y"',
  ])
  assert.deepEqual(
    rows.map((row) => row.verdict),
    ['neo-differs', 'neo-lacks', 'neo-differs'],
  )
  assert.equal(rows[1].gapKey, 'Locator.inputValue')
})

test('real page-goto fixture: expected failure passes; four failures get the observed classes', () => {
  const rows = classifyRuns(fixture('neo'), fixture('reference'))
  assert.deepEqual(countVerdicts(rows), {
    pass: 1,
    'pass via expectLoop': 0,
    'neo-lacks': 1,
    'neo-differs': 0,
    'needs-internals': 3,
    'invalid-here': 0,
    'harness-error': 0,
    skip: 0,
  })
  assert.equal(rows.filter((row) => row.wouldPassWithExpect).length, 2)
  assert.equal(
    rows.find((row) => row.caseId === 'NA-038').gapKey,
    'BrowserContext._options.baseURL',
  )
  const headers = rows.find((row) => row.caseId === 'NA-054')
  assert.equal(headers.gapKey, 'Page.setExtraHTTPHeaders (created context)')
  assert.match(
    headers.evidence[0].firstLine,
    /^Error: page.setExtraHTTPHeaders:/,
  )
  assert.equal(headers.evidence[0].location.line, 761)
  assert.match(headers.evidence[0].location.file, /page-goto.spec.ts$/)
  const expected = rows.find((row) => row.caseId === 'NA-058')
  assert.equal(expected.expectedStatus, 'failed')
  assert.equal(expected.attempts[0].status, 'failed')
  assert.equal(expected.referenceOutcome, 'pass')
  assert.ok(expected.evidence.length > 0)
})

test('precedence: infrastructure, reference, internals, missing API, behavior', () => {
  const errors = [
    {
      results: [
        {
          status: 'failed',
          errors: [
            { message: 'TypeError: locator._expect is not a function' },
            {
              message:
                'Error: worker process exited unexpectedly (code=1, signal=null)',
            },
          ],
        },
      ],
    },
    'TypeError: locator._expect is not a function',
    {
      results: [
        {
          status: 'failed',
          errors: [
            { message: 'TypeError: page.pdf is not a function' },
            { message: 'Error: needs Playwright internals: toImpl' },
          ],
        },
      ],
    },
    {
      results: [
        {
          status: 'failed',
          errors: [
            { message: 'Error: expect(received).toBe(expected)' },
            { message: 'TypeError: page.pdf is not a function' },
          ],
        },
      ],
    },
    'Error: expect(received).toBe(expected)',
  ]
  assert.deepEqual(
    classify(errors, [
      'Error: assertion mismatch',
      'Error: assertion mismatch',
    ]).map((row) => row.verdict),
    [
      'harness-error',
      'invalid-here',
      'needs-internals',
      'neo-lacks',
      'neo-differs',
    ],
  )
})

test('runner status controls retries, skips and unexpected passes', () => {
  const rows = classify([
    {
      status: 'flaky',
      results: [
        { status: 'failed', errors: [{ message: 'Error: flaky assertion' }] },
        { status: 'passed', retry: 1 },
      ],
    },
    { status: 'skipped', expectedStatus: 'skipped', results: [] },
    { expectedStatus: 'failed', results: [{ status: 'passed' }] },
  ])
  assert.deepEqual(
    rows.map((row) => row.verdict),
    ['pass', 'skip', 'neo-differs'],
  )
  assert.equal(rows[0].attempts.length, 2)
  assert.equal(rows[2].gapKey, 'Playwright.expectedStatus')
})

test('missing and interrupted attempts are infrastructure, not a behavioral difference', () => {
  assert.deepEqual(
    classify([
      { results: [] },
      { results: [{ status: 'interrupted' }] },
      { status: undefined, results: [{ status: 'passed' }] },
    ]).map((row) => row.verdict),
    ['harness-error', 'harness-error', 'harness-error'],
  )
})

test('a recovered interruption follows the runner flaky verdict', () => {
  const [row] = classify([
    {
      status: 'flaky',
      results: [{ status: 'interrupted' }, { status: 'passed', retry: 1 }],
    },
  ])
  assert.equal(row.verdict, 'pass')
})

test('expected reference failure is not infrastructure and does not mask a Neo gap', () => {
  const [row] = classify(
    ['TypeError: page.pdf is not a function'],
    [
      {
        status: 'expected',
        expectedStatus: 'failed',
        results: [
          {
            status: 'failed',
            errors: [
              {
                message: "Error: browserType.launch: Executable doesn't exist",
              },
            ],
          },
        ],
      },
    ],
  )
  assert.equal(row.verdict, 'neo-lacks')
})

test('device receiver gaps use the device family', () => {
  const [row] = classify([
    'TypeError: page.keyboard.insertText is not a function',
  ])
  assert.equal(row.gapKey, 'Keyboard.insertText')
})

test('ordinary test timeout and closed-page errors remain behavioral outcomes', () => {
  assert.deepEqual(
    classify([
      'Test timeout of 30000ms exceeded.',
      'Error: page.goto: Target page, context or browser has been closed',
      'Error: options.quality is unsupported for the png screenshots',
    ]).map((row) => row.verdict),
    ['neo-differs', 'neo-differs', 'neo-differs'],
  )
})

test('reference infrastructure failure outranks invalid-here', () => {
  const [row] = classify(
    ['TypeError: page.pdf is not a function'],
    ['Error: worker process exited unexpectedly (code=1)'],
  )
  assert.equal(row.verdict, 'harness-error')
  assert.equal(
    row.referenceEvidence[0].firstLine,
    'Error: worker process exited unexpectedly (code=1)',
  )
})

test('missing/skipped reference matches do not invent invalid-here', () => {
  const [missing] = classifyRuns(
    report(['TypeError: page.pdf is not a function']),
    { suites: [] },
  )
  assert.equal(missing.referenceOutcome, 'missing')
  assert.equal(missing.verdict, 'neo-lacks')
  const [skipped] = classify(
    ['TypeError: page.pdf is not a function'],
    [{ status: 'skipped', results: [] }],
  )
  assert.equal(skipped.verdict, 'neo-lacks')
})

test('unscoped runner errors do not overwrite completed test outcomes', () => {
  const neo = report(
    [{ status: 'expected', results: [{ status: 'passed' }] }],
    [{ message: 'Error: worker process exited unexpectedly' }],
  )
  assert.equal(classifyRuns(neo, report([]))[0].verdict, 'pass')
})

test('recognized Neo receivers produce method gap keys; unrelated JS errors do not', () => {
  const rows = classify([
    'TypeError: handle.asElement(...).contentFrame is not a function',
    'TypeError: page.pdf is not a function',
    'TypeError: userCallback is not a function',
    'TypeError: customer.save is not a function',
  ])
  assert.equal(rows[0].gapKey, 'ElementHandle.contentFrame')
  assert.equal(rows[1].gapKey, 'Page.pdf')
  assert.deepEqual(
    rows.map((row) => row.verdict),
    ['neo-lacks', 'neo-lacks', 'neo-differs', 'neo-differs'],
  )
})

test('private matcher and fixture seams are internals, with a conservative _expect-only count', () => {
  const rows = classify([
    'Error: needs Playwright internals: _expect',
    'Error: Neo does not serve Frame.expect',
    'Error: needs Playwright internals: launch options',
    'TypeError: page._wrapApiCall is not a function',
    'Error: Neo Node harness does not provide Playwright internal fixture toImpl',
    {
      results: [
        {
          status: 'failed',
          errors: [
            { message: 'TypeError: locator._expect is not a function' },
            { message: 'Error: teardown assertion mismatch' },
          ],
        },
      ],
    },
  ])
  assert.ok(rows.every((row) => row.verdict === 'needs-internals'))
  assert.deepEqual(
    rows.map((row) => row.wouldPassWithExpect),
    [true, true, false, false, false, false],
  )
  assert.equal(rows[3].gapKey, 'Page._wrapApiCall')
})

test('baseURL alone and refusal strings inside an assertion diff are not classified by keywords', () => {
  const rows = classify([
    "TypeError: Cannot read properties of undefined (reading 'baseURL')",
    'Error: expect(received).toContain(expected)\nExpected: "Neo does not support signal yet"\nReceived: "different"',
  ])
  assert.ok(rows.every((row) => row.verdict === 'neo-differs'))
})

test('explicit refusal families retain option/context distinctions', () => {
  const rows = classify([
    'Error: browser.newContext: Neo does not support the "proxy" option of a new context yet',
    'Error: page.addInitScript: Neo does not support signal yet',
    'Error: frame.goto: Neo does not serve Frame.goto on child frames yet',
    'Error: browserType.launch: Neo runs attached to BrowserOS; use the provided context / context.browser()',
  ])
  assert.deepEqual(
    rows.map((row) => row.gapKey),
    [
      'Browser.newContext (proxy)',
      'Page.addInitScript (signal)',
      'Frame.goto (child frame)',
      'BrowserType.launch',
    ],
  )
  assert.ok(rows.every((row) => row.verdict === 'neo-lacks'))
})

test('the refusal inventory is source backed and matches its literal examples', () => {
  for (const refusal of refusals.filter((row) => !row.wording.includes('…'))) {
    assert.ok(refusal.sources.length)
    assert.ok(
      refusal.pattern.test(`Error: ${refusal.wording}`),
      refusal.wording,
    )
  }
})

test('suite normalization and full title paths preserve describe and parameter identities', () => {
  const neo = report(['TypeError: page.pdf is not a function'])
  const spec = neo.suites[0].specs[0]
  neo.suites[0].specs = []
  neo.suites[0].suites = ['variant one', 'variant two'].map((title) => ({
    title,
    file: 'page/example.spec.ts',
    specs: [spec],
  }))
  const reference = structuredClone(neo)
  reference.suites[0].file = '/tmp/overlay/tests/page/example.spec.ts'
  const rows = classifyRuns(neo, reference, [
    { file: 'tests/page/example.spec.ts' },
  ])
  assert.equal(rows.length, 2)
  assert.notEqual(rows[0].key, rows[1].key)
  assert.deepEqual(rows[0].titles, ['variant one', 'EX-000 example'])
  assert.ok(rows.every((row) => row.verdict === 'invalid-here'))
})

test('ambiguous multi-project and repeated case identities are rejected', () => {
  const neo = report(['failure'])
  neo.suites[0].specs.push(neo.suites[0].specs[0])
  assert.throws(() => flattenResults(neo), /Duplicate case identity/)
  neo.suites[0].specs.pop()
  neo.suites[0].specs[0].tests.push(neo.suites[0].specs[0].tests[0])
  assert.throws(() => flattenResults(neo), /one project/)
})

test('bridge transitions count selected pass/fail only, without promoting skips or harness errors', () => {
  const rows = classify([
    { status: 'expected', results: [{ status: 'passed' }] },
    'Error: needs Playwright internals: _expect',
    { status: 'skipped', results: [] },
    { results: [{ status: 'interrupted' }] },
    'TypeError: page.pdf is not a function',
  ])
  const bridge = rows.slice(0, 4).map((row, index) => ({
    key: row.key,
    suite: row.suite,
    verdict: ['CleanupBlur', 'BridgeLimit', 'HarnessError', 'HarnessError'][
      index
    ],
  }))
  bridge.push({
    key: caseKey('tests/page/other.spec.ts', ['EX-999 unselected']),
    verdict: 'BridgeLimit',
  })
  const comparison = compareBridge(rows, bridge)
  assert.deepEqual(
    [
      comparison.noVerdictTotal,
      comparison.noVerdictSelected,
      comparison.recovered,
      comparison.unmatched,
    ],
    [5, 4, 2, 1],
  )
  assert.equal(comparison.matrix.BridgeLimit['needs-internals'], 1)
  assert.equal(comparison.matrix['No bridge match']['neo-lacks'], 1)
})

test('a received _expect TypeError and the _apiName refusal are internals; an expected one is not', () => {
  const rows = classify([
    'Error: expect(received).toContain(expected) // indexOf\n\nExpected substring: "waiting for locator(\'option\')"\nReceived string:    "locator._expect is not a function"',
    'Error: toMatchAriaSnapshot can be only used with Page or Locator objects, was called with Page Page {',
    'Error: expect(received).toContain(expected) // indexOf\n\nExpected substring: "locator._expect is not a function"\nReceived string:    "other"',
  ])
  assert.deepEqual(
    rows.map((row) => [row.verdict, row.gapKey, row.wouldPassWithExpect]),
    [
      ['needs-internals', 'Locator._expect', true],
      ['needs-internals', 'Page/Locator._apiName', true],
      ['neo-differs', rows[2].gapKey, false],
    ],
  )
})

test("the Node door's hosted handle cap is a harness error, not a Neo verdict", () => {
  const rows = classify([
    'Error: page.$$eval: neo-runtime: Neo hosted runs keep at most 10000 JS handles; dispose unused handles',
  ])
  assert.equal(rows[0].verdict, 'harness-error')
})

test("a test's own handle variable missing an ElementHandle method, and page.request, are neo-lacks", () => {
  const rows = classify([
    'TypeError: div.scrollIntoViewIfNeeded is not a function',
    {
      results: [
        {
          status: 'failed',
          errors: [
            {
              message:
                "TypeError: Cannot read properties of undefined (reading 'fetch')",
              snippet:
                '> 12 |   const response = await page.request.fetch(server.EMPTY_PAGE);',
            },
          ],
        },
      ],
    },
  ])
  assert.deepEqual(
    rows.map((row) => [row.verdict, row.gapKey]),
    [
      ['neo-lacks', 'ElementHandle.scrollIntoViewIfNeeded'],
      ['neo-lacks', 'Page.request'],
    ],
  )
})
