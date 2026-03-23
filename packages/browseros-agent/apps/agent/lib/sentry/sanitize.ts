/**
 * Sanitize Sentry event data by redacting values at keys that match known
 * sensitive patterns. Used in `beforeSend` to prevent credentials from
 * leaking into error reports.
 */

const REDACTED = '[REDACTED]'

const SENSITIVE_KEY_PATTERNS = [
  'apikey',
  'api_key',
  'accesskeyid',
  'secretaccesskey',
  'sessiontoken',
  'authorization',
  'token',
  'password',
  'secret',
  'credential',
]

function isSensitiveKey(key: string): boolean {
  const lower = key.toLowerCase()
  return SENSITIVE_KEY_PATTERNS.some((p) => lower.includes(p))
}

function sanitize<T>(obj: T): T {
  if (obj === null || obj === undefined) return obj
  if (
    typeof obj === 'string' ||
    typeof obj === 'number' ||
    typeof obj === 'boolean'
  ) {
    return obj
  }
  if (Array.isArray(obj)) {
    return obj.map(sanitize) as T
  }
  if (typeof obj === 'object') {
    const result: Record<string, unknown> = {}
    for (const [key, value] of Object.entries(obj)) {
      result[key] = isSensitiveKey(key) ? REDACTED : sanitize(value)
    }
    return result as T
  }
  return obj
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- Sentry event type varies by SDK
export function sanitizeEvent<E>(event: E): E {
  const e = event as Record<string, any>

  if (Array.isArray(e.breadcrumbs)) {
    e.breadcrumbs = e.breadcrumbs.map((b: Record<string, unknown>) => ({
      ...b,
      data: b.data ? sanitize(b.data) : b.data,
    }))
  }

  if (e.contexts) {
    e.contexts = sanitize(e.contexts)
  }

  if (e.extra) {
    e.extra = sanitize(e.extra)
  }

  for (const value of e.exception?.values ?? []) {
    for (const frame of value.stacktrace?.frames ?? []) {
      if (frame.vars) {
        frame.vars = sanitize(frame.vars)
      }
    }
  }

  return event
}
