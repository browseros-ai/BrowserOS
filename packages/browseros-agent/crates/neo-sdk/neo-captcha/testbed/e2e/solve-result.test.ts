import { expect, test } from 'bun:test'
import type { Verdict } from '../fixtures/catalog'
import {
  classifyReason,
  notSolved,
  parseClawServerSolve,
  parseSdkSolve,
  REASON_VERDICT,
  STATUS_VERDICT,
  stubClawServerSolve,
  stubSdkSolve,
} from './solve-result'

test('tests that the SDK result fields are kept and reason is classified into a test-bed verdict', () => {
  expect(
    parseSdkSolve({
      attempted: false,
      type: 'hcaptcha',
      method: 'none',
      reason: 'unsupported_type',
      message:
        'Neo detected hCaptcha and attempted nothing: it cannot solve this type; Neo cannot tell whether the page is clear — inspect a fresh snapshot and screenshot before continuing, and hand off to the user if a check still blocks it.',
      durationMs: 263,
      paidTasks: 0,
      provider: null,
    }),
  ).toEqual({
    ...notSolved(),
    verdict: 'unsolved',
    attempted: false,
    type: 'hcaptcha',
    method: 'none',
    reason: 'unsupported_type',
    message:
      'Neo detected hCaptcha and attempted nothing: it cannot solve this type; Neo cannot tell whether the page is clear — inspect a fresh snapshot and screenshot before continuing, and hand off to the user if a check still blocks it.',
    durationMs: 263,
    paidTasks: 0,
    provider: null,
  })
  // An unknown reason in an otherwise well-formed payload classifies as `error` with its reason kept.
  expect(
    parseSdkSolve({ attempted: false, method: 'none', reason: 'later' }),
  ).toMatchObject({
    verdict: 'error',
    attempted: false,
    method: 'none',
    reason: 'later',
    error: null,
  })
})

test('tests that every known SDK reason maps to its test-bed verdict and unknown reasons become error', () => {
  const expected: Record<string, Verdict> = {
    auto_passed: 'solved',
    checkbox_passed: 'solved',
    tiles_accepted: 'solved',
    token_accepted: 'solved',
    awaiting_submit: 'token_applied',
    no_captcha: 'already_clear',
    already_passed: 'already_clear',
    not_blocking: 'already_clear',
    unsupported_type: 'unsolved',
    no_provider: 'unsolved',
    provider_unavailable: 'unsolved',
    provider_error: 'unsolved',
    tiles_exhausted: 'unsolved',
    challenge_not_passed: 'unsolved',
    spend_limit_reached: 'unsolved',
    scan_incomplete: 'unknown',
    acceptance_unconfirmed: 'unknown',
    browser_error: 'unknown',
    deadline_exceeded: 'unknown',
    tab_closed: 'cancelled',
    navigated: 'cancelled',
    caller_cancelled: 'cancelled',
  }
  expect(REASON_VERDICT).toEqual(expected)
  for (const [reason, verdict] of Object.entries(expected))
    expect(classifyReason(reason)).toBe(verdict)
  for (const unknown of ['later', '', 'toString', 'constructor'])
    expect(classifyReason(unknown)).toBe('error')
})

test('tests that only the documented fields are copied, never a token', () => {
  const parsed = parseSdkSolve({
    attempted: true,
    type: 'arkose',
    method: 'token',
    reason: 'awaiting_submit',
    message: 'Neo applied an Arkose token',
    durationMs: 9,
    paidTasks: 1,
    provider: 'capsolver',
    token: 'NEVER_REPORT_THIS',
    events: [{ data: { token: 'NEVER_REPORT_THIS' } }],
  })
  expect(JSON.stringify(parsed)).not.toContain('NEVER_REPORT_THIS')
  expect(parsed).toMatchObject({
    verdict: 'token_applied',
    attempted: true,
    method: 'token',
    paidTasks: 1,
    provider: 'capsolver',
    error: null,
  })
})

test('tests that an SDK value carrying a verdict key is rejected as malformed', () => {
  for (const value of [
    { verdict: 'solved', reason: 'auto_passed' },
    {
      verdict: 'solved',
      attempted: true,
      method: 'auto_pass',
      reason: 'auto_passed',
    },
    {
      verdict: undefined,
      attempted: false,
      method: 'none',
      reason: 'no_captcha',
    },
  ])
    expect(parseSdkSolve(value)).toMatchObject({
      verdict: 'error',
      error: 'neo.solveCaptcha returned a verdict',
    })
})

test('tests that a malformed SDK value is an error with safe defaults', () => {
  for (const value of [
    null,
    'solved',
    [],
    { status: 'solved' },
    { attempted: 'true', method: 'none', reason: 'no_captcha' },
    { attempted: true, reason: 'auto_passed' },
    { attempted: true, method: 'auto_pass' },
  ])
    expect(parseSdkSolve(value)).toMatchObject({
      verdict: 'error',
      type: null,
      paidTasks: null,
      error: 'malformed neo.solveCaptcha value',
    })
  expect(
    parseSdkSolve({
      attempted: true,
      method: 'auto_pass',
      reason: 'auto_passed',
      paidTasks: '1',
    }).paidTasks,
  ).toBe(null)
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
    attempted: null,
    type: 'hcaptcha',
    method: 'none',
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
    attempted: null,
    method: null,
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
    attempted: null,
    error: 'page 1 is busy: solve_captcha',
  })
  expect(parseClawServerSolve({ content: [] })).toMatchObject({
    verdict: 'error',
    attempted: null,
    error: 'malformed result',
  })
})

test('tests that the SDK stub follows the new 8-key wire contract and attempted rule', () => {
  const clear = stubSdkSolve('already_clear', null)
  expect(Object.keys(clear)).toEqual([
    'attempted',
    'type',
    'method',
    'reason',
    'message',
    'durationMs',
    'paidTasks',
    'provider',
  ])
  expect(parseSdkSolve(clear)).toMatchObject({
    verdict: 'already_clear',
    attempted: false,
    type: null,
    method: 'none',
    reason: 'no_captcha',
    paidTasks: 0,
    error: null,
  })
  for (const [verdict, reason, attempted, method] of [
    ['solved', 'auto_passed', true, 'auto_pass'],
    ['solved', 'checkbox_passed', true, 'checkbox'],
    ['token_applied', 'awaiting_submit', true, 'token'],
    ['already_clear', 'not_blocking', false, 'none'],
    ['unsolved', 'unsupported_type', false, 'none'],
    ['unsolved', 'no_provider', false, 'none'],
    ['unsolved', 'provider_unavailable', false, 'none'],
    ['unsolved', 'spend_limit_reached', false, 'none'],
    ['unsolved', 'provider_error', true, 'token'],
    ['unsolved', 'challenge_not_passed', true, 'auto_pass'],
    ['unknown', 'scan_incomplete', false, 'none'],
    ['unknown', 'browser_error', false, 'none'],
    ['unknown', 'deadline_exceeded', true, 'auto_pass'],
    ['cancelled', 'navigated', false, 'none'],
  ] as const) {
    const raw = stubSdkSolve(verdict, 'turnstile', reason)
    expect('verdict' in raw).toBe(false)
    expect(raw.message).toContain('snapshot and screenshot')
    expect(parseSdkSolve(raw)).toMatchObject({
      verdict,
      attempted,
      method,
      reason,
      error: null,
    })
  }
})
