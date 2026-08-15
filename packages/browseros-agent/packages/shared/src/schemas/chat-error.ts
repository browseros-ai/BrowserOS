/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Chat error envelope - the single shape describing why an LLM turn failed.
 *
 * The AI SDK's UI message stream can only express an error as
 * `{ type: 'error', errorText: string }` - a strictObject with one string
 * field. Structure therefore travels as JSON serialized into that string, and
 * the same envelope is used for pre-stream HTTP error bodies so both channels
 * parse identically on the client.
 *
 * Intentionally dependency-free rather than a Zod schema like its siblings:
 * apps/app resolves `zod` to v4 while apps/server and this package resolve v3,
 * and unlike the other shared schemas this module is imported as a runtime
 * value on both sides of the wire.
 */

/** Categories the UI can render a tailored action for. */
export const CHAT_ERROR_CODES = [
  'credits_exhausted', // BrowserOS quota spent
  'rate_limited', // provider 429 that is not credit exhaustion
  'auth_failed', // 401/403 - key or token rejected
  'provider_config', // missing/invalid provider setup, caught before streaming
  'context_length', // prompt exceeded the model's window
  'content_filter', // upstream refused the content
  'connection_failed', // provider or agent server unreachable
  'provider_unavailable', // 5xx, overloaded, or upstream outage
  'unknown',
] as const

export type ChatErrorCode = (typeof CHAT_ERROR_CODES)[number]

export interface ChatError {
  code: ChatErrorCode
  /** Card heading, e.g. 'Daily limit reached'. */
  title: string
  /** One user-facing sentence explaining what happened. */
  message: string
  /** Whether retrying the same request could plausibly succeed. */
  retryable: boolean
  provider?: string
  statusCode?: number
  docsUrl?: string
  retryAfterSeconds?: number
  /** Redacted upstream text, shown behind a disclosure. */
  details?: string
}

export interface ChatErrorEnvelope {
  error: ChatError
}

const CHAT_ERROR_CODE_SET: ReadonlySet<string> = new Set(CHAT_ERROR_CODES)

export function isChatErrorCode(value: unknown): value is ChatErrorCode {
  return typeof value === 'string' && CHAT_ERROR_CODE_SET.has(value)
}

export function serializeChatError(error: ChatError): string {
  return JSON.stringify({ error } satisfies ChatErrorEnvelope)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

/**
 * Parse an envelope out of an error string. Returns null for anything that is
 * not a well-formed envelope - plain prose, a foreign JSON body, or an older
 * server that still sends bare messages.
 */
export function parseChatErrorEnvelope(raw: string): ChatError | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }

  if (!isRecord(parsed) || !isRecord(parsed.error)) return null

  const error = parsed.error
  if (!isChatErrorCode(error.code)) return null
  if (typeof error.message !== 'string' || typeof error.title !== 'string') {
    return null
  }

  return {
    code: error.code,
    title: error.title,
    message: error.message,
    retryable: error.retryable === true,
    provider: optionalString(error.provider),
    statusCode: optionalNumber(error.statusCode),
    docsUrl: optionalString(error.docsUrl),
    retryAfterSeconds: optionalNumber(error.retryAfterSeconds),
    details: optionalString(error.details),
  }
}

/**
 * Best-effort human-readable text for consumers that render an error string
 * directly instead of building a card from the envelope.
 */
export function chatErrorMessage(raw: string): string {
  return parseChatErrorEnvelope(raw)?.message ?? raw
}
