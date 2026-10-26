import { expect, test } from 'bun:test'
import type { CaptchaPage } from '../fixtures/catalog'
import { probes } from './probes'
import { classifyOutcome, exitCode, renderTable, runCaptchaE2E } from './runner'
import { startStubMcp } from './stub-mcp'

const verdict = {
  submitSelector: '#submit',
  acceptedPattern: '^ACCEPTED',
  rejectedPattern: '^REJECTED',
}
const pages: CaptchaPage[] = [
  {
    id: 'solved',
    title: 'Solved',
    kind: 'fixture',
    family: 'turnstile',
    path: '/solved.html',
    expect: ['solved'],
    mayNeedPaid: false,
    ready: 'true',
    verdict,
  },
  {
    id: 'clear',
    title: 'Clear',
    kind: 'fixture',
    family: 'none',
    path: '/clear.html',
    expect: ['already_clear'],
    mayNeedPaid: false,
    ready: 'true',
    verdict,
  },
  {
    id: 'blocked',
    title: 'Blocked',
    kind: 'fixture',
    family: 'turnstile',
    path: '/blocked.html',
    expect: ['failed', 'needs_human'],
    mayNeedPaid: false,
    ready: 'true',
    verdict,
  },
  {
    id: 'unsupported',
    title: 'Unsupported',
    kind: 'demo',
    family: 'hcaptcha',
    url: 'https://demo.invalid',
    expect: ['unsupported', 'needs_human'],
    mayNeedPaid: false,
    ready: 'true',
  },
]
const tokenLengthExpression = () => '0'

async function run(
  scenario: Parameters<typeof startStubMcp>[0]['scenario'],
  overrides = {},
  stubPages = pages,
) {
  const stub = await startStubMcp({ scenario, pages: stubPages })
  try {
    return await runCaptchaE2E({
      mcpUrl: stub.url,
      fixtureOrigin: 'http://fixtures.invalid',
      pages,
      tokenLengthExpression,
      probeTimeoutMs: 40,
      pollIntervalMs: 1,
      ...overrides,
    })
  } finally {
    await stub.stop()
  }
}

test('tests that contract results all pass with independent acceptance and exit zero', async () => {
  const records = await run('contract')
  expect(records.map((r) => r.outcome)).toEqual([
    'PASS',
    'PASS',
    'PASS',
    'PASS',
  ])
  expect(records[0]).toMatchObject({
    verdict: 'accepted',
    callbackFired: true,
    tokenLength: 123,
  })
  expect(records[2]).toMatchObject({
    status: 'failed',
    verdict: 'rejected',
    outcome: 'PASS',
  })
  expect(exitCode(records)).toBe(0)
  expect(records.every((r) => r.latencyMs >= 0)).toBe(true)
})

test('tests that a token_applied stub passes after the site accepts submission', async () => {
  const records = await run('token-applied')
  expect(records[0]).toMatchObject({
    status: 'token_applied',
    method: 'token',
    reason: 'awaiting_submit',
    events: ['captcha.token_applied'],
    verdict: 'accepted',
    tokenLength: 123,
    callbackFired: true,
    outcome: 'PASS',
    note: 'token applied; site accepted on submit',
  })
  expect(records.every((record) => record.outcome === 'PASS')).toBe(true)
  expect(exitCode(records)).toBe(0)
})

for (const [bodyText, verdict, note] of [
  ['REJECTED: invalid-token', 'rejected', 'false positive'],
  [
    'waiting for verification',
    'error',
    'independent check failed; verdict timed out',
  ],
] as const) {
  test(`tests that token_applied fails through the runner when the verdict is ${verdict}`, async () => {
    const selected = [pages[0] as CaptchaPage]
    const stub = await startStubMcp({
      scenario: 'token-applied',
      pages: selected,
      bodyText,
    })
    try {
      const records = await runCaptchaE2E({
        mcpUrl: stub.url,
        fixtureOrigin: 'http://fixtures.invalid',
        pages: selected,
        tokenLengthExpression,
        probeTimeoutMs: 20,
        pollIntervalMs: 1,
      })
      expect(records[0]).toMatchObject({
        status: 'token_applied',
        verdict,
        outcome: 'FAIL',
        note,
      })
      expect(exitCode(records)).toBe(1)
    } finally {
      await stub.stop()
    }
  })
}

for (const [acceptance, outcome, note] of [
  ['accepted', 'PASS', 'token applied; site accepted on submit'],
  ['rejected', 'FAIL', 'false positive'],
  ['error', 'FAIL', 'independent check failed'],
  ['n/a', 'FAIL', 'independent check failed'],
] as const) {
  test(`tests that token_applied with a ${acceptance} verdict gets ${outcome}`, () => {
    expect(
      classifyOutcome(
        pages[0] as CaptchaPage,
        'token_applied',
        acceptance,
        'awaiting_submit',
      ),
    ).toEqual({ outcome, note })
  })
}

test('tests that token_applied keeps expectation and missing-tool rules', () => {
  for (const page of pages.slice(1)) {
    expect(
      classifyOutcome(page, 'token_applied', 'accepted', 'awaiting_submit'),
    ).toEqual({ outcome: 'FAIL', note: '' })
  }
  expect(
    classifyOutcome(
      pages[0] as CaptchaPage,
      'token_applied',
      'accepted',
      'awaiting_submit',
      { expectToolMissing: true },
    ),
  ).toEqual({ outcome: 'FAIL', note: '' })
})

test('tests that the table shows token_applied and its reason alongside the outcome note', async () => {
  const records = await run('contract')
  const first = records[0]
  if (!first) throw new Error('missing result')
  const record = {
    ...first,
    status: 'token_applied',
    reason: 'awaiting_submit',
    note: 'token applied; site accepted on submit',
  }
  const table = renderTable([record])
  expect(table).toContain('| solved | solved | token_applied |')
  expect(table).toContain(
    'awaiting_submit; token applied; site accepted on submit |',
  )
  expect(renderTable([{ ...record, note: '' }])).toContain(
    '| awaiting_submit |',
  )
  expect(renderTable([{ ...record, reason: 'n/a' }])).not.toContain(
    'n/a; token applied',
  )
})

test('tests that contract verdicts cover demo success messages and JSON success responses', async () => {
  const demos = [
    'Verification Success',
    '"success"\\s*:\\s*true',
    'reCAPTCHA passed successfully!',
  ].map(
    (acceptedPattern, index): CaptchaPage => ({
      ...(pages[0] as CaptchaPage),
      id: `demo-${index}`,
      family: index === 1 ? 'turnstile' : 'recaptcha_v2',
      kind: 'demo',
      path: undefined,
      url: `https://demo.invalid/${index}`,
      verdict: { ...verdict, acceptedPattern },
    }),
  )
  const records = await run('contract', { pages: demos }, demos)
  expect(records.map((r) => r.verdict)).toEqual([
    'accepted',
    'accepted',
    'accepted',
  ])
})

test('tests that a missing tool fails unless explicitly expected', async () => {
  const records = await run('no-tool')
  expect(
    records.every((r) => r.status === 'tool_not_found' && r.outcome === 'FAIL'),
  ).toBe(true)
  expect(exitCode(records)).toBe(1)
  const expected = await run('no-tool', { expectToolMissing: true })
  expect(expected.every((r) => r.outcome === 'PASS')).toBe(true)
  expect(exitCode(expected)).toBe(0)
  expect(
    (await run('contract', { expectToolMissing: true })).every(
      (r) => r.outcome === 'FAIL',
    ),
  ).toBe(true)
})

test('tests that solved with a rejected verdict fails as a false positive', async () => {
  const records = await run('false-positive')
  expect(records[0]).toMatchObject({
    status: 'solved',
    verdict: 'rejected',
    outcome: 'FAIL',
  })
  expect(records[0]?.note).toContain('false positive')
  expect(exitCode(records)).toBe(1)
})

for (const failure of ['submit', 'timeout']) {
  test(`tests that a ${failure} failure rejects an unconfirmed solve and retains the probe note`, async () => {
    const stub = await startStubMcp({
      scenario: 'contract',
      pages: [pages[0] as CaptchaPage],
      bodyText: 'waiting for verification',
    })
    const proxy = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      async fetch(request) {
        if (request.method === 'POST' && failure === 'submit') {
          const rpc = await request.clone().json()
          if (
            rpc.params?.name === 'evaluate' &&
            String(rpc.params.arguments?.code).startsWith(probes.submit)
          )
            return Response.json({
              jsonrpc: '2.0',
              id: rpc.id,
              result: {
                isError: true,
                content: [{ type: 'text', text: 'submit control missing' }],
              },
            })
        }
        return fetch(new Request(stub.url, request))
      },
    })
    try {
      const records = await runCaptchaE2E({
        mcpUrl: `http://127.0.0.1:${proxy.port}`,
        fixtureOrigin: 'http://fixtures.invalid',
        pages: [pages[0] as CaptchaPage],
        tokenLengthExpression,
        probeTimeoutMs: 20,
        pollIntervalMs: 1,
      })
      expect(records[0]).toMatchObject({
        status: 'solved',
        verdict: 'error',
        outcome: 'FAIL',
      })
      expect(records[0]?.note).toBe(
        `independent check failed; ${failure === 'submit' ? 'verdict check failed' : 'verdict timed out'}`,
      )
      expect(exitCode(records)).toBe(1)
    } finally {
      await proxy.stop(true)
      await stub.stop()
    }
  })
}

test('tests that a rejection beyond the evaluate inline limit still fails a solved result', async () => {
  const large = {
    ...(pages[0] as CaptchaPage),
    verdict: {
      ...verdict,
      acceptedPattern: 'ACCEPTED',
      rejectedPattern: 'REJECTED',
    },
  }
  const stub = await startStubMcp({
    scenario: 'contract',
    pages: [large],
    bodyText: `${'page text '.repeat(1000)}REJECTED`,
  })
  try {
    const records = await runCaptchaE2E({
      mcpUrl: stub.url,
      fixtureOrigin: 'http://fixtures.invalid',
      pages: [large],
      tokenLengthExpression,
      probeTimeoutMs: 30,
      pollIntervalMs: 1,
    })
    expect(records[0]).toMatchObject({
      status: 'solved',
      verdict: 'rejected',
      outcome: 'FAIL',
    })
  } finally {
    await stub.stop()
  }
})

test('tests that unpaid provider failure is unverified and paid failure fails', async () => {
  const paid = { ...(pages[0] as CaptchaPage), mayNeedPaid: true }
  const failed: CaptchaPage = { ...paid, expect: ['needs_human'] }
  const records = await run('contract', { pages: [paid] }, [failed])
  expect(records[0]).toMatchObject({
    outcome: 'UNVERIFIED',
    status: 'needs_human',
  })
  expect(records[0]?.note).toContain('paid path unverified: provider_error')
  expect(exitCode(records)).toBe(0)
  expect(
    (await run('contract', { pages: [paid], paid: true }, [failed]))[0]
      ?.outcome,
  ).toBe('FAIL')
})

test('tests that classification follows all four outcome rules in order', () => {
  const check = (status: string, acceptance: string, options = {}) =>
    classifyOutcome(
      { ...(pages[0] as CaptchaPage), mayNeedPaid: true },
      status,
      acceptance,
      'denied',
      options,
    )
  expect(
    check('tool_not_found', 'error', { expectToolMissing: true }).outcome,
  ).toBe('PASS')
  expect(check('solved', 'accepted', { expectToolMissing: true }).outcome).toBe(
    'FAIL',
  )
  expect(check('solved', 'n/a')).toEqual({
    outcome: 'PASS',
    note: 'no independent check',
  })
  expect(check('solved', 'rejected').outcome).toBe('FAIL')
  expect(check('solved', 'error')).toEqual({
    outcome: 'FAIL',
    note: 'independent check failed',
  })
  expect(check('needs_human', 'rejected').outcome).toBe('UNVERIFIED')
  expect(check('cancelled', 'rejected').outcome).toBe('FAIL')
  expect(check('error', 'error').outcome).toBe('FAIL')
})

test('tests that already_clear on a CAPTCHA needs independent acceptance', () => {
  for (const family of ['turnstile', 'recaptcha_v2', 'hcaptcha'] as const) {
    const page: CaptchaPage = {
      ...(pages[0] as CaptchaPage),
      family,
      expect: ['already_clear'],
    }
    expect(
      classifyOutcome(page, 'already_clear', 'accepted', 'already_passed'),
    ).toEqual({ outcome: 'PASS', note: '' })
    expect(
      classifyOutcome(page, 'already_clear', 'rejected', 'already_passed'),
    ).toEqual({ outcome: 'FAIL', note: 'false positive' })
    expect(
      classifyOutcome(page, 'already_clear', 'error', 'already_passed'),
    ).toEqual({ outcome: 'FAIL', note: 'independent check failed' })
    expect(
      classifyOutcome(page, 'already_clear', 'n/a', 'already_passed'),
    ).toEqual({ outcome: 'PASS', note: 'no independent check' })
  }
})

test('tests that already_clear on a non-CAPTCHA page keeps its existing outcome', () => {
  const page: CaptchaPage = {
    ...(pages[1] as CaptchaPage),
    expect: ['already_clear'],
  }
  for (const verdict of ['accepted', 'rejected', 'error', 'n/a'])
    expect(
      classifyOutcome(page, 'already_clear', verdict, 'no_captcha'),
    ).toEqual({ outcome: 'PASS', note: '' })
})

test('tests that a widget timeout is an error and pages still close', async () => {
  const unready = [{ ...(pages[0] as CaptchaPage), ready: 'false' }]
  const records = await run('contract', { pages: unready }, unready)
  expect(records[0]).toMatchObject({
    status: 'error',
    outcome: 'FAIL',
    note: 'widget never rendered',
  })
})

test('tests that submission still runs after a widget timeout and every opened page closes', async () => {
  const unready = [{ ...(pages[0] as CaptchaPage), ready: 'false' }]
  const stub = await startStubMcp({ scenario: 'contract', pages: unready })
  try {
    await runCaptchaE2E({
      mcpUrl: stub.url,
      fixtureOrigin: 'http://fixtures.invalid',
      pages: unready,
      tokenLengthExpression,
      probeTimeoutMs: 20,
      pollIntervalMs: 1,
    })
    expect(
      stub.calls.some(
        (call) =>
          call.name === 'evaluate' &&
          String(call.args.code).startsWith(probes.submit),
      ),
    ).toBe(true)
    expect(
      stub.calls.filter(
        (call) => call.name === 'tabs' && call.args.action === 'close',
      ),
    ).toHaveLength(1)
  } finally {
    await stub.stop()
  }
})

test('tests that Markdown reports escape cells and contain the required columns', async () => {
  const records = await run('contract')
  const first = records[0]
  if (!first) throw new Error('missing result')
  first.note = 'a|b\nc'
  const table = renderTable(records)
  expect(table).toContain(
    '| Page | Expected | Result | Type | Path | Verdict | Latency | Cost | Outcome | Note |',
  )
  expect(table).toContain('a\\|b c')
  expect(table).toContain('already_clear')
  expect(table).not.toContain('NEVER_REPORT_THIS')
})
