import type { ApiError } from '@browseros/claw-api'

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
