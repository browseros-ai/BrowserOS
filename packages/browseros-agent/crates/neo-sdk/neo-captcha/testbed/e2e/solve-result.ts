import type { McpToolResult } from '../../../../../contracts/claw-mcp/tests/mcp-client'
import type { Verdict } from '../fixtures/catalog'

/** One solve, as the report keeps it. Never a token: only these fields are copied. */
export interface SolveResult {
  /**
   * The test bed's own classification of the SDK's `reason` (or claw-server's status mapped as
   * neo-captcha maps it); `error` when the call failed, was malformed, or returned an unknown
   * reason; `n/a` when it was not made. The Neo SDK itself returns no `verdict`.
   */
  verdict: string
  /** claw-server's own status; `null` for the SDK, which returns neither status nor verdict. */
  status: string | null
  /** Whether the SDK attempted a solve; `null` for claw-server or a failed/unmade call. */
  attempted: boolean | null
  type: string | null
  /** The solver method (`none`, `auto_pass`, `checkbox`, `tiles`, `token`), or `null`. */
  method: string | null
  reason: string
  message: string
  durationMs: number | null
  paidTasks: number | null
  provider: 'capsolver' | '2captcha' | null
  costUsd: number | null
  /** Why the call failed, e.g. `host/captcha_unavailable`. */
  error: string | null
}

export const notSolved = (): SolveResult => ({
  verdict: 'n/a',
  status: null,
  attempted: null,
  type: null,
  method: null,
  reason: 'n/a',
  message: '',
  durationMs: null,
  paidTasks: null,
  provider: null,
  costUsd: null,
  error: null,
})

const object = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
const word = (value: unknown) => (typeof value === 'string' ? value : 'n/a')
const provider = (value: unknown) =>
  value === 'capsolver' || value === '2captcha' ? value : null
const number = (value: unknown) =>
  typeof value === 'number' && Number.isFinite(value) ? value : null
const clip = (text: string) =>
  text.length > 300 ? `${text.slice(0, 300)}…` : text

/**
 * The test bed's own classification of `neo.solveCaptcha`'s `reason`, kept next to the parser so
 * the catalog's `expect: Verdict[]` and the runner's outcome rules work without a wire `verdict`.
 */
export const REASON_VERDICT: Record<string, Verdict> = {
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

/** Classifies an SDK `reason` into the test bed's verdict; unknown reasons become `error`. */
export function classifyReason(reason: string): Verdict | 'error' {
  return Object.hasOwn(REASON_VERDICT, reason)
    ? (REASON_VERDICT[reason] as Verdict)
    : 'error'
}

/**
 * Parses `neo.solveCaptcha(page)`'s value (`{ attempted, type, method, reason, message,
 * durationMs, paidTasks, provider }`) and classifies `reason` into the test bed's own `verdict`.
 * A value that still carries a `verdict` key is rejected as malformed.
 */
export function parseSdkSolve(value: unknown): SolveResult {
  const data = object(value)
  const hasVerdict = Object.hasOwn(data, 'verdict')
  const wellFormed =
    !hasVerdict &&
    typeof data.attempted === 'boolean' &&
    typeof data.reason === 'string' &&
    typeof data.method === 'string'
  const reason = word(data.reason)
  return {
    ...notSolved(),
    verdict: wellFormed ? classifyReason(reason) : 'error',
    attempted: typeof data.attempted === 'boolean' ? data.attempted : null,
    type: typeof data.type === 'string' ? data.type : null,
    method: typeof data.method === 'string' ? data.method : null,
    reason,
    message: typeof data.message === 'string' ? clip(data.message) : '',
    durationMs: number(data.durationMs),
    paidTasks: number(data.paidTasks),
    provider: provider(data.provider),
    error: hasVerdict
      ? 'neo.solveCaptcha returned a verdict'
      : wellFormed
        ? null
        : 'malformed neo.solveCaptcha value',
  }
}

/** Mapping from claw-server's closed `status` enum into the test bed's own `Verdict`. */
export const STATUS_VERDICT: Record<string, Verdict> = {
  solved: 'solved',
  token_applied: 'token_applied',
  already_clear: 'already_clear',
  unsupported: 'unsolved',
  needs_human: 'unsolved',
  cancelled: 'cancelled',
  failed: 'unknown',
}

/** claw-server's `solve_captcha` result. */
export function parseClawServerSolve(result: McpToolResult): SolveResult {
  const text = result.content[0]?.text ?? ''
  let data: Record<string, unknown> = {}
  try {
    data = object(JSON.parse(text))
  } catch {}
  const status = word(data.status)
  const known = !result.isError && Object.hasOwn(STATUS_VERDICT, status)
  const type = data.type ?? data.captcha_type ?? object(data.captcha).type
  return {
    verdict: known ? (STATUS_VERDICT[status] as Verdict) : 'error',
    status: known ? status : 'error',
    attempted: null,
    type: typeof type === 'string' ? type : null,
    method: typeof data.method === 'string' ? data.method : null,
    reason: word(data.reason),
    message: '',
    durationMs: number(
      data.durationMs ?? data.duration_ms ?? data.elapsedMs ?? data.elapsed_ms,
    ),
    paidTasks: number(data.paidTasks ?? data.paid_tasks),
    provider: provider(data.provider),
    costUsd: number(data.costUsd ?? data.cost_usd),
    error: known ? null : clip(result.isError ? text : 'malformed result'),
  }
}

const STUB_REASON: Record<Verdict, string> = {
  solved: 'auto_passed',
  token_applied: 'awaiting_submit',
  already_clear: 'no_captcha',
  unsolved: 'provider_error',
  cancelled: 'navigated',
  unknown: 'browser_error',
}

const STUB_METHOD: Record<string, string> = {
  auto_passed: 'auto_pass',
  checkbox_passed: 'checkbox',
  tiles_accepted: 'tiles',
  token_accepted: 'token',
  awaiting_submit: 'token',
  no_captcha: 'none',
  already_passed: 'none',
  not_blocking: 'none',
  unsupported_type: 'none',
  no_provider: 'none',
  provider_unavailable: 'none',
  provider_error: 'token',
  tiles_exhausted: 'tiles',
  challenge_not_passed: 'auto_pass',
  spend_limit_reached: 'none',
  scan_incomplete: 'none',
  acceptance_unconfirmed: 'token',
  browser_error: 'none',
  deadline_exceeded: 'auto_pass',
  tab_closed: 'none',
  navigated: 'none',
  caller_cancelled: 'none',
}

const NEVER_ATTEMPTED = new Set([
  'no_captcha',
  'not_blocking',
  'already_passed',
  'unsupported_type',
  'scan_incomplete',
  'no_provider',
  'provider_unavailable',
  'spend_limit_reached',
])

const ALWAYS_ATTEMPTED = new Set([
  'auto_passed',
  'checkbox_passed',
  'tiles_accepted',
  'token_accepted',
  'awaiting_submit',
  'acceptance_unconfirmed',
  'tiles_exhausted',
  'challenge_not_passed',
  'deadline_exceeded',
])

/**
 * The SDK's 8-key wire value for a test-bed verdict (and optional pinned reason), kept beside the
 * parser so the local stub follows the exact wire contract and `attempted` rule.
 */
export function stubSdkSolve(
  verdict: Verdict,
  type: string | null,
  reason?: string,
) {
  const resolvedReason = reason ?? STUB_REASON[verdict]
  const method = STUB_METHOD[resolvedReason] ?? 'none'
  // Zero-spend stub (`acted = false`, `attempts.token = 0`): matches the wire contract's
  // exhaustive `attempted` rule by reason and method.
  const attempted = NEVER_ATTEMPTED.has(resolvedReason)
    ? false
    : ALWAYS_ATTEMPTED.has(resolvedReason)
      ? true
      : method !== 'none'
  const closing = attempted
    ? 'Neo cannot confirm site acceptance — inspect a fresh snapshot and screenshot before continuing, retry neo.solveCaptcha(page) if still blocked, or hand off to the user.'
    : 'Neo cannot tell whether the page is clear — inspect a fresh snapshot and screenshot before continuing, and hand off to the user if a check still blocks it.'
  return {
    attempted,
    type,
    method,
    reason: resolvedReason,
    message: `Neo stubbed ${resolvedReason}; ${closing}`,
    durationMs: 1,
    paidTasks: 0,
    provider: null,
  }
}

/** claw-server's terminal payload for a status. */
export function stubClawServerSolve(
  status: string,
  type: string | null,
  /** The page's expected reason, as the SDK stub answers; else the status's usual one. */
  reason?: string,
): McpToolResult {
  const reasons: Record<string, string> = {
    solved: 'auto_passed',
    token_applied: 'awaiting_submit',
    already_clear: 'no_captcha',
    unsupported: 'unsupported_type',
    failed: 'browser_error',
    cancelled: 'caller_cancelled',
    needs_human: 'provider_error',
  }
  const value = {
    status,
    type,
    method:
      status === 'token_applied'
        ? 'token'
        : status === 'solved'
          ? 'auto_pass'
          : 'none',
    reason: reason ?? reasons[status],
    durationMs: 1,
    paidTasks: 0,
    provider: null,
    costUsd: 0,
    events:
      status === 'token_applied'
        ? [{ event: 'captcha.token_applied', data: { callbackRan: true } }]
        : [],
  }
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
    structuredContent: value,
  }
}
