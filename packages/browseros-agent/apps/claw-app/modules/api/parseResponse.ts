/**
 * Throws a structured ApiError on non-OK responses so React Query
 * treats the request as failed. The error carries the status code
 * and parsed body so toasts / screen-level fallbacks can branch on
 * shape without re-reading the response.
 */
type ApiError = Error & {
  status: number
  body: unknown
}

export async function parseResponse<T>(response: Response): Promise<T> {
  if (response.ok) {
    return (await response.json()) as T
  }
  let body: unknown
  try {
    body = await response.json()
  } catch {
    body = await response.text().catch(() => '')
  }
  const err = new Error(extractMessage(body, response.status)) as ApiError
  err.status = response.status
  err.body = body
  throw err
}

function extractMessage(body: unknown, status: number): string {
  if (
    body &&
    typeof body === 'object' &&
    'error' in body &&
    typeof (body as { error: unknown }).error === 'string'
  ) {
    return (body as { error: string }).error
  }
  return `Request failed with status ${status}`
}
