/** Normalizes the legacy server's error dialects at the client seam. */
export class ServerApiError extends Error {
  override readonly name = 'ServerApiError'
  readonly status: number
  readonly code?: string

  constructor(error: { status: number; code?: string; message: string }) {
    super(error.message)
    this.status = error.status
    this.code = error.code
  }

  static async fromResponse(response: Response): Promise<ServerApiError> {
    const raw = await response.clone().text()
    let body: unknown
    try {
      body = JSON.parse(raw)
    } catch {
      body = raw
    }
    const outer = isRecord(body) ? body : undefined
    const nested = outer?.error
    const error = isRecord(nested) ? nested : outer
    const message =
      stringValue(error?.message) ??
      stringValue(nested) ??
      stringValue(body) ??
      `${response.status} ${response.statusText}`.trim()
    const code = error?.code
    return new ServerApiError({
      status: response.status,
      message,
      ...(typeof code === 'string' || typeof code === 'number'
        ? { code: String(code) }
        : {}),
    })
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}
