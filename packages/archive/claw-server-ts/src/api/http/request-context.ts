import type { Context, MiddlewareHandler } from 'hono'

/**
 * Per-request id for the canonical API surface. Minted server-side —
 * never read from an inbound header — stored on the Hono context,
 * echoed back as `x-request-id`, and embedded in canonical `ApiError`
 * bodies so a reported failure can be tied to the exact request.
 */
export type RequestContextEnv = {
  Variables: {
    requestId: string
  }
}

export const requestIdMiddleware: MiddlewareHandler<RequestContextEnv> = async (
  c,
  next,
) => {
  const requestId = crypto.randomUUID()
  c.set('requestId', requestId)
  await next()
  c.header('x-request-id', requestId)
}

export function requestIdFor(c: Context<RequestContextEnv>): string {
  return c.get('requestId')
}
