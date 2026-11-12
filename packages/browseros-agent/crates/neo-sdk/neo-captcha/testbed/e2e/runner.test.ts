import { expect, test } from 'bun:test'
import {
  type CaptchaPage,
  FIXTURE_PAGES,
  SPOTTING_PAGES,
} from '../fixtures/catalog'
import type { Server } from './preflight'
import {
  classifyOutcome,
  exitCode,
  progressLine,
  type ResultRecord,
  renderSummary,
  renderTable,
  runCaptchaE2E,
  summarize,
} from './runner'
import { parseStepTag } from './sdk-page'
import { startStubMcp } from './stub-mcp'

const site = {
  submitSelector: '#submit',
  acceptedPattern: '^ACCEPTED',
  rejectedPattern: '^REJECTED',
}
const solvePage: CaptchaPage = {
  id: 'solved',
  title: 'Solved',
  kind: 'fixture',
  family: 'turnstile',
  path: '/solved.html',
  expect: ['solved'],
  mayNeedPaid: false,
  ready: 'true',
  site,
}
const refusalPage: CaptchaPage = {
  id: 'unsupported',
  title: 'Unsupported',
  kind: 'demo',
  family: 'hcaptcha',
  url: 'https://demo.invalid/',
  expect: ['unsolved'],
  expectType: 'hcaptcha',
  expectReason: ['unsupported_type'],
  mayNeedPaid: false,
  ready: 'true',
}
const solve = (verdict: string, extra = {}) => ({
  verdict,
  type: 'turnstile',
  reason: 'auto_passed',
  paidTasks: 0,
  error: null,
  ...extra,
})
const tokenLengthExpression = () => '0'

async function run(
  scenario: Parameters<typeof startStubMcp>[0]['scenario'],
  options: {
    server?: Server
    pages?: CaptchaPage[]
    stubPages?: CaptchaPage[]
    bodyText?: string
    paid?: boolean
    expectToolMissing?: boolean
  } = {},
) {
  const pages = options.pages ?? [...FIXTURE_PAGES, ...SPOTTING_PAGES]
  const stub = await startStubMcp({
    scenario,
    server: options.server,
    pages: options.stubPages ?? pages,
    bodyText: options.bodyText,
  })
  try {
    const records = await runCaptchaE2E({
      mcpUrl: `${stub.url}/mcp`,
      server: options.server ?? 'sdk',
      fixtureOrigin: 'http://fixtures.invalid',
      pages,
      tokenLengthExpression,
      paid: options.paid,
      expectToolMissing: options.expectToolMissing,
      probeTimeoutMs: 40,
      pollIntervalMs: 1,
    })
    return { records, calls: stub.calls }
  } finally {
    await stub.stop()
  }
}

test('tests that a matching verdict cannot hide a wrong CAPTCHA type', () => {
  expect(
    classifyOutcome(
      refusalPage,
      solve('unsolved', { type: 'hcaptcha', reason: 'unsupported_type' }),
      'n/a',
    ).outcome,
  ).toBe('PASS')
  for (const type of ['geetest', null]) {
    const result = classifyOutcome(
      refusalPage,
      solve('unsolved', { type, reason: 'unsupported_type' }),
      'n/a',
    )
    expect(result.outcome).toBe('FAIL')
    expect(result.note).toContain('expected type hcaptcha')
  }
})

test('tests that a detection refused for another reason is not an unsupported refusal', () => {
  expect(
    classifyOutcome(
      refusalPage,
      solve('unsolved', { type: 'hcaptcha', reason: 'provider_error' }),
      'n/a',
    ),
  ).toEqual({
    outcome: 'FAIL',
    note: 'expected reason unsupported_type, received provider_error',
  })
})

test('tests that paid task counts cannot exceed the page budget or go missing', () => {
  const page = { ...refusalPage, maxPaidTasks: 0 }
  const unsolved = (paidTasks: number | null) =>
    solve('unsolved', {
      type: 'hcaptcha',
      reason: 'unsupported_type',
      paidTasks,
    })
  expect(classifyOutcome(page, unsolved(0), 'n/a').outcome).toBe('PASS')
  expect(classifyOutcome(page, unsolved(1), 'n/a')).toEqual({
    outcome: 'FAIL',
    note: 'paid tasks 1 exceed cap 0',
  })
  expect(classifyOutcome(page, unsolved(null), 'n/a').note).toBe(
    'paid task count missing',
  )
  expect(
    classifyOutcome({ ...page, maxPaidTasks: undefined }, unsolved(10), 'n/a')
      .outcome,
  ).toBe('PASS')
})

for (const [acceptance, outcome, note] of [
  ['accepted', 'PASS', 'token applied; site accepted on submit'],
  ['rejected', 'FAIL', 'false positive'],
  ['error', 'FAIL', 'independent check failed'],
  ['n/a', 'FAIL', 'independent check failed'],
] as const) {
  test(`tests that token_applied for a solve page with a ${acceptance} site gets ${outcome}`, () => {
    expect(
      classifyOutcome(solvePage, solve('token_applied'), acceptance),
    ).toEqual({ outcome, note })
  })
}

test('tests that a token delivery with no site check passes but is never called solved', () => {
  const delivery: CaptchaPage = {
    ...refusalPage,
    expect: ['token_applied'],
    expectType: 'arkose',
    expectReason: undefined,
    maxPaidTasks: 1,
    mayNeedPaid: true,
  }
  expect(
    classifyOutcome(
      delivery,
      solve('token_applied', { type: 'arkose', paidTasks: 1 }),
      'n/a',
    ),
  ).toEqual({
    outcome: 'PASS',
    note: 'token delivered; no site check, not independently solved',
  })
})

test('tests that solved needs the site and already_clear needs it only on a CAPTCHA page', () => {
  expect(classifyOutcome(solvePage, solve('solved'), 'accepted')).toEqual({
    outcome: 'PASS',
    note: '',
  })
  expect(classifyOutcome(solvePage, solve('solved'), 'rejected')).toEqual({
    outcome: 'FAIL',
    note: 'false positive',
  })
  expect(classifyOutcome(solvePage, solve('solved'), 'error').note).toBe(
    'independent check failed',
  )
  expect(classifyOutcome(solvePage, solve('solved'), 'n/a')).toEqual({
    outcome: 'PASS',
    note: 'no independent check',
  })
  const clear = { ...solvePage, expect: ['already_clear' as const] }
  expect(
    classifyOutcome(clear, solve('already_clear'), 'rejected').outcome,
  ).toBe('FAIL')
  for (const verdict of ['accepted', 'rejected', 'error', 'n/a'])
    expect(
      classifyOutcome(
        { ...clear, family: 'none' },
        solve('already_clear', { type: null, reason: 'no_captcha' }),
        verdict,
      ),
    ).toEqual({ outcome: 'PASS', note: '' })
})

test('tests that a site accepting a page called unsolved proves the solver wrong', () => {
  const block = {
    ...solvePage,
    expect: ['unsolved' as const, 'unknown' as const],
  }
  expect(classifyOutcome(block, solve('unsolved'), 'rejected').outcome).toBe(
    'PASS',
  )
  expect(classifyOutcome(block, solve('unsolved'), 'accepted')).toEqual({
    outcome: 'FAIL',
    note: 'the site accepted a page the solver called unsolved',
  })
  expect(classifyOutcome(block, solve('unknown'), 'accepted').outcome).toBe(
    'PASS',
  )
  // Not even an unpaid demo that expects a solve may call the contradiction unverified.
  const demo = { ...solvePage, mayNeedPaid: true }
  expect(
    classifyOutcome(
      demo,
      solve('unsolved', { reason: 'provider_error' }),
      'accepted',
    ),
  ).toEqual({
    outcome: 'FAIL',
    note: 'the site accepted a page the solver called unsolved',
  })
  expect(
    classifyOutcome(
      demo,
      solve('unsolved', { reason: 'provider_error' }),
      'error',
    ).outcome,
  ).toBe('UNVERIFIED')
})

test('tests that an absent solver passes only when absence is expected', () => {
  const absent = (code: string) =>
    solve('error', { type: null, reason: 'n/a', error: code, paidTasks: null })
  for (const code of ['host/captcha_unavailable', 'mcp/tool_not_found'])
    expect(
      classifyOutcome(solvePage, absent(code), 'rejected', {
        expectToolMissing: true,
      }),
    ).toEqual({ outcome: 'PASS', note: 'solver absent, as expected' })
  expect(
    classifyOutcome(solvePage, solve('solved'), 'accepted', {
      expectToolMissing: true,
    }),
  ).toEqual({
    outcome: 'FAIL',
    note: 'the solver answered solved, but --expect-tool-missing was set',
  })
  expect(
    classifyOutcome(solvePage, absent('script/page_busy'), 'n/a', {
      expectToolMissing: true,
    }).outcome,
  ).toBe('FAIL')
  expect(
    classifyOutcome(solvePage, absent('host/captcha_unavailable'), 'n/a'),
  ).toEqual({
    outcome: 'FAIL',
    note: 'solve failed: host/captcha_unavailable',
  })
})

test('tests that unpaid provider failures are unverified and paid runs fail them', () => {
  const paid = { ...solvePage, mayNeedPaid: true }
  const failed = solve('unsolved', { reason: 'provider_error' })
  expect(classifyOutcome(paid, failed, 'rejected')).toEqual({
    outcome: 'UNVERIFIED',
    note: 'paid path unverified: provider_error',
  })
  expect(classifyOutcome(paid, solve('unknown'), 'error').outcome).toBe(
    'UNVERIFIED',
  )
  expect(
    classifyOutcome(paid, failed, 'rejected', { paid: true }).outcome,
  ).toBe('FAIL')
  expect(classifyOutcome(paid, solve('cancelled'), 'error').outcome).toBe(
    'FAIL',
  )
  expect(
    classifyOutcome(paid, solve('n/a', { type: null }), 'error').outcome,
  ).toBe('FAIL')
})

for (const server of ['sdk', 'claw-server'] as const) {
  test(`tests that every fixture and spotting page passes against the ${server} contract`, async () => {
    const { records, calls } = await run('contract', { server })
    expect(records.filter((r) => r.outcome !== 'PASS')).toEqual([])
    expect(exitCode(records)).toBe(0)
    expect(summarize(records)).toMatchObject({
      pass: 19,
      siteAccepted: 5,
      deliveredOnly: 0,
      detected: 14,
      paidTasks: 0,
    })
    const recaptcha = records.find((r) => r.id === 'recaptcha-v2-test')
    expect(recaptcha).toMatchObject({
      verdict: 'solved',
      status: server === 'sdk' ? null : 'solved',
      site: 'accepted',
      tokenLength: 123,
      callbackFired: true,
    })
    expect(records.find((r) => r.id === 'hcaptcha-test')).toMatchObject({
      verdict: 'unsolved',
      type: 'hcaptcha',
      reason: 'unsupported_type',
      check: 'detect',
      site: 'rejected',
    })
    const closes = calls.filter((call) =>
      server === 'sdk'
        ? call.step === 'close'
        : call.name === 'tabs' && call.args.action === 'close',
    )
    expect(closes).toHaveLength(records.length)
  })
}

test('tests that SDK scripts run in a task per page and never call claw-server tools', async () => {
  const { calls } = await run('contract', {
    pages: [FIXTURE_PAGES[0] as CaptchaPage],
  })
  expect(calls.every((call) => call.name === 'run')).toBe(true)
  expect(calls.map((call) => call.step)).toEqual([
    'open',
    'goto',
    'ready',
    'solve',
    'observe',
    'submit',
    'site',
    'close',
  ])
  const solveCall = calls.find((call) => call.step === 'solve')
  expect(String(solveCall?.args.code)).toContain(
    'return await neo.solveCaptcha(page);',
  )
  expect(solveCall?.args.timeout).toBe(150_000)
  expect(parseStepTag(String(solveCall?.args.code))?.args).toMatchObject({
    task: 'stub-task-1',
    tab: 1,
  })
})

test('tests that solved with a rejected site verdict fails as a false positive', async () => {
  const { records } = await run('false-positive', { pages: [solvePage] })
  expect(records[0]).toMatchObject({
    verdict: 'solved',
    site: 'rejected',
    outcome: 'FAIL',
    note: 'false positive',
  })
  expect(exitCode(records)).toBe(1)
})

test('tests that a token_applied verdict passes once the site accepts the submission', async () => {
  const { records } = await run('token-applied', { pages: [solvePage] })
  expect(records[0]).toMatchObject({
    verdict: 'token_applied',
    reason: 'awaiting_submit',
    site: 'accepted',
    outcome: 'PASS',
    note: 'token applied; site accepted on submit',
  })
})

for (const [bodyText, siteVerdict, note] of [
  ['REJECTED: invalid-token', 'rejected', 'false positive'],
  [
    'waiting for verification',
    'error',
    'independent check failed; site verdict timed out',
  ],
] as const) {
  test(`tests that token_applied fails through the runner when the site verdict is ${siteVerdict}`, async () => {
    const { records } = await run('token-applied', {
      pages: [solvePage],
      bodyText,
    })
    expect(records[0]).toMatchObject({
      verdict: 'token_applied',
      site: siteVerdict,
      outcome: 'FAIL',
      note,
    })
  })
}

test('tests that an absent SDK solver passes every page only when absence is expected', async () => {
  const expected = await run('no-tool', { expectToolMissing: true })
  expect(expected.records.every((r) => r.outcome === 'PASS')).toBe(true)
  expect(summarize(expected.records)).toMatchObject({
    pass: 19,
    siteAccepted: 0,
    deliveredOnly: 0,
    detected: 0,
    absent: 19,
  })
  expect(renderSummary(expected.records)).toContain(
    'solver absent as expected: 19',
  )
  expect(
    expected.records.every((r) => r.error === 'host/captcha_unavailable'),
  ).toBe(true)
  expect(expected.calls.some((call) => call.step === 'ready')).toBe(false)
  const unexpected = await run('no-tool', { pages: [solvePage] })
  expect(unexpected.records[0]).toMatchObject({
    verdict: 'error',
    outcome: 'FAIL',
    note: 'solve failed: host/captcha_unavailable',
  })
  const present = await run('contract', {
    pages: [solvePage],
    expectToolMissing: true,
  })
  expect(present.records[0]?.outcome).toBe('FAIL')
})

test('tests that an absent claw-server tool is recorded as tool_not_found', async () => {
  const { records } = await run('no-tool', {
    server: 'claw-server',
    pages: [solvePage],
    expectToolMissing: true,
  })
  expect(records[0]).toMatchObject({
    verdict: 'error',
    status: 'tool_not_found',
    error: 'mcp/tool_not_found',
    outcome: 'PASS',
  })
})

test('tests that a widget that never renders fails, still checks the site and closes its page', async () => {
  const unready = { ...solvePage, ready: 'false' }
  const { records, calls } = await run('contract', { pages: [unready] })
  expect(records[0]).toMatchObject({
    verdict: 'n/a',
    outcome: 'FAIL',
    note: 'widget never rendered',
  })
  expect(calls.map((call) => call.step)).toEqual([
    'open',
    'goto',
    'ready',
    'observe',
    'submit',
    'site',
    'close',
  ])
})

test('tests that a failed submit keeps the solve unconfirmed', async () => {
  const stub = await startStubMcp({ scenario: 'contract', pages: [solvePage] })
  const proxy = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      if (request.method === 'POST') {
        const rpc = await request.clone().json()
        if (
          parseStepTag(String(rpc.params?.arguments?.code ?? ''))?.step ===
          'submit'
        )
          return Response.json({
            jsonrpc: '2.0',
            id: rpc.id,
            result: {
              isError: true,
              content: [{ type: 'text', text: 'script/no_match' }],
              structuredContent: {
                error: { owner: 'script', code: 'no_match', message: 'none' },
              },
            },
          })
      }
      return fetch(new Request(stub.url, request))
    },
  })
  try {
    const records = await runCaptchaE2E({
      mcpUrl: `http://127.0.0.1:${proxy.port}/mcp`,
      server: 'sdk',
      fixtureOrigin: 'http://fixtures.invalid',
      pages: [solvePage],
      tokenLengthExpression,
      probeTimeoutMs: 20,
    })
    expect(records[0]).toMatchObject({
      verdict: 'solved',
      site: 'error',
      outcome: 'FAIL',
      note: 'independent check failed; site check failed',
    })
  } finally {
    await proxy.stop(true)
    await stub.stop()
  }
})

test('tests that a rejection beyond the inline limit still fails a solved result', async () => {
  const large = {
    ...solvePage,
    site: { ...site, acceptedPattern: 'ACCEPTED', rejectedPattern: 'REJECTED' },
  }
  const { records } = await run('contract', {
    pages: [large],
    bodyText: `${'page text '.repeat(1000)}REJECTED`,
  })
  expect(records[0]).toMatchObject({
    verdict: 'solved',
    site: 'rejected',
    outcome: 'FAIL',
  })
})

test('tests that the table and summary keep the exact verdict, reason and paid tasks apart from site acceptance', async () => {
  const { records } = await run('contract', {
    pages: [solvePage, refusalPage],
  })
  const first = records[0] as ResultRecord
  first.note = 'a|b\nc'
  const table = renderTable(records)
  expect(table).toContain(
    '| Page | Check | Expected | Verdict | Type | Reason | Paid | Site | Latency | Outcome | Note |',
  )
  expect(table).toContain('a\\|b c')
  expect(table).toContain(
    '| unsupported | detect | unsolved | unsolved | hcaptcha | unsupported_type | 0 | n/a |',
  )
  expect(
    renderTable([{ ...first, verdict: 'unsolved', status: 'needs_human' }]),
  ).toContain('| unsolved (needs_human) |')
  expect(renderSummary(records)).toBe(
    'PASS=2 FAIL=0 UNVERIFIED=0 · site-accepted submissions: 1 · token deliveries without a site check: 0 · detections and refusals: 1 · paid tasks: 0',
  )
})

test('tests that progress lines carry the exact answer and the outcome', async () => {
  const seen: string[] = []
  const stub = await startStubMcp({
    scenario: 'contract',
    pages: [refusalPage],
  })
  try {
    await runCaptchaE2E({
      mcpUrl: stub.url,
      server: 'sdk',
      fixtureOrigin: 'http://fixtures.invalid',
      pages: [refusalPage],
      tokenLengthExpression,
      onRecord: (record) => seen.push(progressLine(record)),
    })
  } finally {
    await stub.stop()
  }
  expect(seen).toEqual([
    '[captcha-e2e] unsupported: unsolved unsupported_type type=hcaptcha paid=0 site=n/a -> PASS',
  ])
})

test('tests that a type refusal fails unless the page allows it, and is never unverified', () => {
  const refusal = solve('unsolved', { reason: 'unsupported_type' })
  const block = {
    ...solvePage,
    expect: ['unsolved' as const, 'unknown' as const],
  }
  expect(classifyOutcome(block, refusal, 'rejected')).toEqual({
    outcome: 'FAIL',
    note: 'Neo refused this CAPTCHA type as unsupported',
  })
  const demo = { ...solvePage, mayNeedPaid: true }
  expect(classifyOutcome(demo, refusal, 'error').outcome).toBe('FAIL')
  const spotting = {
    ...block,
    expect: ['unsolved' as const],
    allowUnsupported: true,
  }
  expect(classifyOutcome(spotting, refusal, 'n/a').outcome).toBe('PASS')
  expect(
    classifyOutcome(
      spotting,
      solve('unsolved', { reason: 'challenge_not_passed' }),
      'n/a',
    ).outcome,
  ).toBe('PASS')
  expect(
    classifyOutcome(
      refusalPage,
      solve('unsolved', { type: 'hcaptcha', reason: 'unsupported_type' }),
      'n/a',
    ).outcome,
  ).toBe('PASS')
})

test('tests that a failed solve may have paid, so it is counted as unreported, unlike an absent solver', () => {
  const base = {
    outcome: 'FAIL' as const,
    check: 'solve' as const,
    site: 'error' as const,
    paidTasks: null,
  }
  const records = [
    { ...base, verdict: 'error', error: 'script/run_timeout' },
    { ...base, verdict: 'error', error: 'host/captcha_unavailable' },
    { ...base, verdict: 'n/a', error: null },
    { ...base, verdict: 'solved', error: null, paidTasks: 2 },
  ] as unknown as ResultRecord[]
  expect(summarize(records)).toMatchObject({
    paidTasks: 2,
    paidTasksUnreported: 1,
  })
  expect(renderSummary(records)).toContain(
    'paid tasks: 2 (+1 results without a count)',
  )
})

test('tests that the SDK submits by clicking the form button inside the page', async () => {
  const { calls } = await run('contract', { pages: [solvePage] })
  const submit = String(calls.find((call) => call.step === 'submit')?.args.code)
  expect(submit).toContain('return await page.evaluate(() => {')
  expect(submit).toContain('button.click()')
})
