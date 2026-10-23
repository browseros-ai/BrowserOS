import type { McpToolResult } from '../../claw-mcp/tests/mcp-client'

const statuses = new Set([
  'solved',
  'token_applied',
  'already_clear',
  'unsupported',
  'failed',
  'cancelled',
  'needs_human',
])
const object = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === 'object'
    ? (value as Record<string, unknown>)
    : {}
const word = (value: unknown) => (typeof value === 'string' ? value : 'n/a')
const number = (value: unknown) =>
  typeof value === 'number' && Number.isFinite(value) ? value : null

/** The only module that knows the solver's output contract. Never copy tokens. */
export function parseSolveResult(result: McpToolResult) {
  const text = result.content[0]?.text ?? ''
  let data: Record<string, unknown> = {}
  try {
    data = object(JSON.parse(text))
  } catch {}
  const status = word(data.status)
  return {
    status: !result.isError && statuses.has(status) ? status : 'error',
    type: word(data.type ?? data.captcha_type ?? object(data.captcha).type),
    method: word(data.method),
    reason: word(data.reason),
    requestId: word(data.requestId ?? data.request_id),
    toolElapsedMs: number(
      data.durationMs ?? data.duration_ms ?? data.elapsedMs ?? data.elapsed_ms,
    ),
    costUsd: number(data.costUsd ?? data.cost_usd),
    paidTasks: number(data.paidTasks ?? data.paid_tasks),
    events: Array.isArray(data.events)
      ? data.events
          .map((event) =>
            typeof event === 'string' ? event : object(event).event,
          )
          .filter((event): event is string => typeof event === 'string')
      : [],
    note: result.isError ? text : '',
  }
}

/** Keep stub terminal payloads beside the parser so contract changes stay here. */
export function stubSolveResult(status: string, type: string): McpToolResult {
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
    type: type === 'none' ? null : type,
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
