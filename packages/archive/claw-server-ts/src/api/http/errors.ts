import type { ApiError } from '@browseros/claw-api'
import type { Context } from 'hono'
import type { RequestContextEnv } from './request-context'

/**
 * Error envelope for the canonical API surface. Route handlers and the
 * server-level catch-all both funnel through here so error bodies stay
 * in lockstep with the contract's `ApiError` schema.
 */
export function canonicalApiError(
  code: string,
  message: string,
  requestId?: string,
): ApiError {
  return requestId === undefined
    ? { code, message }
    : { code, message, requestId }
}

export function apiError(
  c: Context<RequestContextEnv, string>,
  status: 400 | 403 | 404 | 409 | 410 | 413,
  code: string,
  message: string,
) {
  return c.json(canonicalApiError(code, message, c.get('requestId')), status)
}
