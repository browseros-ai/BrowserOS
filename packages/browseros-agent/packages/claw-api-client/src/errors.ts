export class ApiResponseError extends Error {
  override name = 'ApiResponseError'
  readonly code?: string
  /** The API's own message, written to be shown to people. */
  readonly apiMessage?: string
  readonly requestId?: string
  /** Every other field of the error body, such as `computers` on `key_full`. */
  readonly details?: Readonly<Record<string, unknown>>

  /** `body` is the error body openapi-fetch already parsed, so `response` stays unread. */
  constructor(
    public readonly response: Response,
    body?: unknown,
  ) {
    super(
      `BrowserOS neo API request failed with status ${response.status.toString()}`,
    )
    if (!isApiErrorBody(body)) return
    const { code, message, requestId, ...details } = body
    this.code = code
    this.apiMessage = message
    if (typeof requestId === 'string') this.requestId = requestId
    if (Object.keys(details).length > 0) this.details = details
  }
}

function isApiErrorBody(
  body: unknown,
): body is { code: string; message: string } & Record<string, unknown> {
  if (typeof body !== 'object' || body === null || Array.isArray(body)) {
    return false
  }
  const { code, message } = body as Record<string, unknown>
  return typeof code === 'string' && typeof message === 'string'
}
