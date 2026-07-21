import type { AcpRuntimeTurnResultError } from 'acpx/runtime'

export interface AcpxErrorOptions {
  code: string
  retryable?: boolean
  cause?: unknown
}

export class AcpxError extends Error {
  readonly code: string
  readonly retryable: boolean

  constructor(message: string, opts: AcpxErrorOptions) {
    super(message, opts.cause !== undefined ? { cause: opts.cause } : undefined)
    this.name = 'AcpxError'
    this.code = opts.code
    this.retryable = opts.retryable ?? false
  }
}

export class AcpxAgentNotFoundError extends AcpxError {
  constructor(message: string, opts?: { cause?: unknown; doctor?: string }) {
    const body = opts?.doctor ? `${message}\n\n${opts.doctor}` : message
    super(body, {
      code: 'agent_not_found',
      retryable: false,
      cause: opts?.cause,
    })
    this.name = 'AcpxAgentNotFoundError'
  }
}

const DEFAULT_AUTH_MESSAGE =
  'Authentication required. Set the relevant ACPX_AUTH_<METHOD> env var or populate ~/.acpx/config.json with credentials before retrying.'

export class AcpxAuthRequiredError extends AcpxError {
  constructor(message?: string, opts?: { cause?: unknown }) {
    super(message ?? DEFAULT_AUTH_MESSAGE, {
      code: 'auth_required',
      retryable: false,
      cause: opts?.cause,
    })
    this.name = 'AcpxAuthRequiredError'
  }
}

export class AcpxTurnTimeoutError extends AcpxError {
  constructor(message: string, opts?: { cause?: unknown }) {
    super(message, {
      code: 'turn_timeout',
      retryable: true,
      cause: opts?.cause,
    })
    this.name = 'AcpxTurnTimeoutError'
  }
}

const AGENT_NOT_FOUND_HINTS = [
  'agent_not_found',
  'acp_backend_missing',
  'agent not found',
]
const AUTH_HINTS = ['auth_required', 'unauthenticated', 'authentication']
const TIMEOUT_HINTS = ['timeout', 'timed out']

function includesAny(haystack: string, hints: readonly string[]): boolean {
  const lower = haystack.toLowerCase()
  return hints.some((hint) => lower.includes(hint))
}

export function fromRuntimeError(err: AcpRuntimeTurnResultError): AcpxError {
  const haystack = `${err.code ?? ''} ${err.message}`

  if (includesAny(haystack, AGENT_NOT_FOUND_HINTS)) {
    return new AcpxAgentNotFoundError(err.message, { cause: err })
  }
  if (includesAny(haystack, AUTH_HINTS)) {
    return new AcpxAuthRequiredError(err.message, { cause: err })
  }
  if (includesAny(haystack, TIMEOUT_HINTS)) {
    return new AcpxTurnTimeoutError(err.message, { cause: err })
  }
  return new AcpxError(err.message, {
    code: err.code ?? 'unknown',
    retryable: err.retryable ?? false,
    cause: err,
  })
}
