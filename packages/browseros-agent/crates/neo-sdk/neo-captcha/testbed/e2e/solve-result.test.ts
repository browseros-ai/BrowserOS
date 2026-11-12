import { expect, test } from 'bun:test'
import {
  notSolved,
  parseClawServerSolve,
  parseSdkSolve,
  STATUS_VERDICT,
  stubClawServerSolve,
  stubSdkSolve,
} from './solve-result'

test('tests that the SDK verdict and reason are kept exactly as returned', () => {
  expect(
    parseSdkSolve({
      verdict: 'unsolved',
      type: 'hcaptcha',
      reason: 'unsupported_type',
      message:
        'The CAPTCHA still blocks the page (Neo cannot solve this CAPTCHA type); stop and tell the user it needs their help.',
      durationMs: 263,
      paidTasks: 0,
    }),
  ).toEqual({
    ...notSolved(),
    verdict: 'unsolved',
    type: 'hcaptcha',
    reason: 'unsupported_type',
    message:
      'The CAPTCHA still blocks the page (Neo cannot solve this CAPTCHA type); stop and tell the user it needs their help.',
    durationMs: 263,
    paidTasks: 0,
  })
  // A verdict this runner does not know yet is still reported as the SDK said it.
  expect(parseSdkSolve({ verdict: 'deferred', reason: 'later' })).toMatchObject(
    { verdict: 'deferred', reason: 'later', error: null },
  )
})

test('tests that only the documented fields are copied, never a token', () => {
  const parsed = parseSdkSolve({
    verdict: 'token_applied',
    type: 'arkose',
    reason: 'awaiting_submit',
    message: 'A CAPTCHA token is in the page',
    durationMs: 9,
    paidTasks: 1,
    token: 'NEVER_REPORT_THIS',
    events: [{ data: { token: 'NEVER_REPORT_THIS' } }],
  })
  expect(JSON.stringify(parsed)).not.toContain('NEVER_REPORT_THIS')
  expect(parsed).toMatchObject({ verdict: 'token_applied', paidTasks: 1 })
})

test('tests that a malformed SDK value is an error with safe defaults', () => {
  for (const value of [null, 'solved', { status: 'solved' }])
    expect(parseSdkSolve(value)).toMatchObject({
      verdict: 'error',
      type: null,
      paidTasks: null,
      error: 'malformed neo.solveCaptcha value',
    })
  expect(parseSdkSolve({ verdict: 'solved', paidTasks: '1' }).paidTasks).toBe(
    null,
  )
})

test('tests that claw-server statuses map as neo-captcha maps them and stay recorded', () => {
  expect(STATUS_VERDICT).toEqual({
    solved: 'solved',
    token_applied: 'token_applied',
    already_clear: 'already_clear',
    unsupported: 'unsolved',
    needs_human: 'unsolved',
    cancelled: 'cancelled',
    failed: 'unknown',
  })
  expect(
    parseClawServerSolve(stubClawServerSolve('unsupported', 'hcaptcha')),
  ).toMatchObject({
    verdict: 'unsolved',
    status: 'unsupported',
    type: 'hcaptcha',
    reason: 'unsupported_type',
    paidTasks: 0,
    costUsd: 0,
    error: null,
  })
})

test('tests that claw-server snake_case fields are read and tool errors stay errors', () => {
  expect(
    parseClawServerSolve({
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            status: 'needs_human',
            captcha: { type: 'recaptcha_v2' },
            elapsed_ms: 12,
            cost_usd: 0,
            paid_tasks: 2,
          }),
        },
      ],
    }),
  ).toMatchObject({
    verdict: 'unsolved',
    type: 'recaptcha_v2',
    durationMs: 12,
    costUsd: 0,
    paidTasks: 2,
  })
  expect(
    parseClawServerSolve({
      content: [{ type: 'text', text: 'page 1 is busy: solve_captcha' }],
      isError: true,
    }),
  ).toMatchObject({
    verdict: 'error',
    status: 'error',
    error: 'page 1 is busy: solve_captcha',
  })
  expect(parseClawServerSolve({ content: [] })).toMatchObject({
    verdict: 'error',
    error: 'malformed result',
  })
})

test('tests that the SDK stub follows the SDK value contract', () => {
  expect(parseSdkSolve(stubSdkSolve('already_clear', null))).toMatchObject({
    verdict: 'already_clear',
    type: null,
    reason: 'no_captcha',
    paidTasks: 0,
    error: null,
  })
})
