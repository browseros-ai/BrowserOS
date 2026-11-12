import type { McpToolResult } from '../../../../../contracts/claw-mcp/tests/mcp-client'
import type { Verdict } from '../fixtures/catalog'

/** One solve, as the report keeps it. Never a token: only these fields are copied. */
export interface SolveResult {
  /** The SDK's verdict exactly; claw-server's status mapped as neo-captcha maps it; `error` when
   * the call failed; `n/a` when it was not made. */
  verdict: string
  /** claw-server's own status; `null` for the SDK, which returns a verdict. */
  status: string | null
  type: string | null
  reason: string
  message: string
  durationMs: number | null
  paidTasks: number | null
  costUsd: number | null
  /** Why the call failed, e.g. `host/captcha_unavailable`. */
  error: string | null
}

export const notSolved = (): SolveResult => ({
  verdict: 'n/a',
  status: null,
  type: null,
  reason: 'n/a',
  message: '',
  durationMs: null,
  paidTasks: null,
  costUsd: null,
  error: null,
})

const object = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === 'object'
    ? (value as Record<string, unknown>)
    : {}
const word = (value: unknown) => (typeof value === 'string' ? value : 'n/a')
const number = (value: unknown) =>
  typeof value === 'number' && Number.isFinite(value) ? value : null
const clip = (text: string) =>
  text.length > 300 ? `${text.slice(0, 300)}…` : text

/** `neo.solveCaptcha(page)`'s value, field by field as the SDK returned it. */
export function parseSdkSolve(value: unknown): SolveResult {
  const data = object(value)
  return {
    ...notSolved(),
    verdict: typeof data.verdict === 'string' ? data.verdict : 'error',
    type: typeof data.type === 'string' ? data.type : null,
    reason: word(data.reason),
    message: typeof data.message === 'string' ? clip(data.message) : '',
    durationMs: number(data.durationMs),
    paidTasks: number(data.paidTasks),
    error:
      typeof data.verdict === 'string'
        ? null
        : 'malformed neo.solveCaptcha value',
  }
}

/** neo-captcha's `SolveStatus::verdict`, the one mapping from claw-server's status. */
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
    type: typeof type === 'string' ? type : null,
    reason: word(data.reason),
    message: '',
    durationMs: number(
      data.durationMs ?? data.duration_ms ?? data.elapsedMs ?? data.elapsed_ms,
    ),
    paidTasks: number(data.paidTasks ?? data.paid_tasks),
    costUsd: number(data.costUsd ?? data.cost_usd),
    error: known ? null : clip(result.isError ? text : 'malformed result'),
  }
}

/** The SDK's value for a verdict, beside the parser so the stub follows the same contract. */
export function stubSdkSolve(verdict: Verdict, type: string | null) {
  const reasons: Record<Verdict, string> = {
    solved: 'auto_passed',
    token_applied: 'awaiting_submit',
    already_clear: 'no_captcha',
    unsolved: 'unsupported_type',
    cancelled: 'navigated',
    unknown: 'browser_error',
  }
  return {
    verdict,
    type,
    reason: reasons[verdict],
    message: `stub ${verdict}`,
    durationMs: 1,
    paidTasks: 0,
  }
}

/** claw-server's terminal payload for a status. */
export function stubClawServerSolve(
  status: string,
  type: string | null,
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
    reason: reasons[status],
    durationMs: 1,
    paidTasks: 0,
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
