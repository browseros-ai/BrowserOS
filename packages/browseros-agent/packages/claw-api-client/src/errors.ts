export class ApiResponseError extends Error {
  override name = 'ApiResponseError'

  constructor(public readonly response: Response) {
    super(
      `BrowserOS neo API request failed with status ${response.status.toString()}`,
    )
  }
}

/**
 * The server's own explanation for a failed request, when it sent one.
 *
 * `ApiResponseError.message` carries only the status, which is all a generic
 * caller needs. Where the server has something specific to say, such as a
 * provider refusing a credential, the reason is in the response body and the
 * status alone hides exactly the part the user needs. The response held by the
 * error is a clone, so reading it here does not disturb anything.
 */
export async function apiErrorReason(error: unknown): Promise<string | null> {
  if (!(error instanceof ApiResponseError)) return null
  try {
    const body: unknown = await error.response.clone().json()
    if (body && typeof body === 'object' && 'message' in body) {
      const message = (body as { message?: unknown }).message
      if (typeof message === 'string' && message.trim() !== '') return message
    }
    return null
  } catch {
    // No body, or not JSON. The status in `error.message` is all there is.
    return null
  }
}
