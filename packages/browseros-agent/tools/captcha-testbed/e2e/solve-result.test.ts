import { expect, test } from 'bun:test'
import { parseSolveResult, stubSolveResult } from './solve-result'

test('tests that token_applied preserves the v1.4 status and event without event data', () => {
  const parsed = parseSolveResult({
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          status: 'token_applied',
          method: 'token',
          reason: 'awaiting_submit',
          events: [
            {
              event: 'captcha.token_applied',
              data: { token: 'NEVER_REPORT_THIS' },
            },
          ],
        }),
      },
    ],
  })
  expect(parsed).toMatchObject({
    status: 'token_applied',
    method: 'token',
    reason: 'awaiting_submit',
    events: ['captcha.token_applied'],
  })
  expect(JSON.stringify(parsed)).not.toContain('NEVER_REPORT_THIS')
})

test('tests that the token_applied stub follows contract v1.4', () => {
  expect(
    parseSolveResult(stubSolveResult('token_applied', 'recaptcha_v2')),
  ).toMatchObject({
    status: 'token_applied',
    type: 'recaptcha_v2',
    method: 'token',
    reason: 'awaiting_submit',
    events: ['captcha.token_applied'],
  })
})

test('tests that stub terminal reasons match the v1.1 contract', () => {
  for (const [status, reason] of [
    ['solved', 'auto_passed'],
    ['already_clear', 'no_captcha'],
    ['unsupported', 'unsupported_type'],
    ['failed', 'browser_error'],
    ['cancelled', 'caller_cancelled'],
    ['needs_human', 'provider_error'],
  ] as const) {
    expect(
      parseSolveResult(stubSolveResult(status, 'turnstile')),
    ).toMatchObject({
      status,
      reason,
      method: status === 'solved' ? 'auto_pass' : 'none',
    })
  }
  expect(parseSolveResult(stubSolveResult('already_clear', 'none')).type).toBe(
    'n/a',
  )
})

test('tests that camelCase results preserve only report fields and event names', () => {
  expect(
    parseSolveResult({
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            status: 'solved',
            type: 'turnstile',
            method: 'token',
            reason: 'ok',
            requestId: 'request-1',
            durationMs: 42,
            costUsd: 0.002,
            paidTasks: 1,
            events: [
              { event: 'captcha.solved', data: { token: 'NEVER_REPORT_THIS' } },
            ],
            token: 'NEVER_REPORT_THIS',
          }),
        },
      ],
      structuredContent: { status: 'failed' },
    }),
  ).toEqual({
    status: 'solved',
    type: 'turnstile',
    method: 'token',
    reason: 'ok',
    requestId: 'request-1',
    toolElapsedMs: 42,
    costUsd: 0.002,
    paidTasks: 1,
    events: ['captcha.solved'],
    note: '',
  })
})

test('tests that snake_case fields and text JSON are accepted', () => {
  expect(
    parseSolveResult({
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            status: 'needs_human',
            captcha: { type: 'recaptcha_v2' },
            method: 'none',
            request_id: 'r2',
            elapsed_ms: 12,
            cost_usd: 0,
          }),
        },
      ],
    }),
  ).toMatchObject({
    status: 'needs_human',
    type: 'recaptcha_v2',
    requestId: 'r2',
    toolElapsedMs: 12,
    costUsd: 0,
  })
})

test('tests that missing and invalid fields have safe defaults', () => {
  expect(parseSolveResult({ content: [] })).toEqual({
    status: 'error',
    type: 'n/a',
    method: 'n/a',
    reason: 'n/a',
    requestId: 'n/a',
    toolElapsedMs: null,
    costUsd: null,
    paidTasks: null,
    events: [],
    note: '',
  })
  expect(
    parseSolveResult({ content: [{ type: 'text', text: 'not JSON' }] }).status,
  ).toBe('error')
  expect(
    parseSolveResult({
      content: [],
      structuredContent: { status: 'invented', costUsd: 'secret' },
    }).costUsd,
  ).toBeNull()
})

test('tests that only the first text block is parsed and tool errors remain errors', () => {
  expect(
    parseSolveResult({
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            status: 'already_clear',
            type: null,
            paid_tasks: 0,
            duration_ms: 8,
          }),
        },
        { type: 'text', text: 'Tip: rename your session' },
      ],
    }),
  ).toMatchObject({
    status: 'already_clear',
    type: 'n/a',
    paidTasks: 0,
    toolElapsedMs: 8,
  })
  expect(
    parseSolveResult({
      content: [{ type: 'text', text: JSON.stringify({ status: 'solved' }) }],
      isError: true,
    }),
  ).toMatchObject({ status: 'error' })
  for (const text of [
    'solve_captcha: page 99 does not exist',
    'page 1 is busy: solve_captcha',
  ]) {
    expect(
      parseSolveResult({ content: [{ type: 'text', text }], isError: true }),
    ).toMatchObject({ status: 'error', note: text })
  }
  expect(
    parseSolveResult({ content: [], structuredContent: { status: 'solved' } })
      .status,
  ).toBe('error')
})
