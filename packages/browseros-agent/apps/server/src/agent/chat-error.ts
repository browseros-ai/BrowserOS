/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Classifies a failed LLM turn into the shared ChatError envelope.
 *
 * The AI SDK masks stream errors by default (`onError = () => "An error
 * occurred."`) to avoid leaking server internals. Here the "server" is a
 * loopback process on the user's own machine using the user's own credentials,
 * so the mask costs diagnosis without buying isolation - we classify instead,
 * and keep raw upstream text redacted and capped behind `details`.
 */

import {
  AISDKError,
  APICallError,
  InvalidPromptError,
  LoadAPIKeyError,
  NoSuchModelError,
} from '@ai-sdk/provider'
import {
  type ChatError,
  serializeChatError,
} from '@browseros/shared/schemas/chat-error'
import { RetryError } from 'ai'

export interface ChatErrorContext {
  provider?: string
}

const DETAILS_MAX_LENGTH = 500

const USAGE_DOCS_URL = 'https://dub.sh/browseros-usage-limit'
const USAGE_PAGE_URL = '/app.html#/settings/usage'
const CONNECTION_DOCS_URL =
  'https://docs.browseros.com/troubleshooting/connection-issues'

/**
 * Key-shaped tokens get scrubbed out of free-text upstream messages. The shared
 * Sentry sanitizer only redacts by object key, so it cannot help here.
 */
const SECRET_PATTERNS: RegExp[] = [
  /\b(?:sk|pk|rk)-[A-Za-z0-9_-]{12,}/g,
  /\bBearer\s+[A-Za-z0-9._-]{12,}/gi,
  /\bAKIA[0-9A-Z]{16}\b/g,
  /\bgh[pousr]_[A-Za-z0-9]{20,}/g,
]

function redact(text: string): string {
  return SECRET_PATTERNS.reduce(
    (acc, pattern) => acc.replace(pattern, '[REDACTED]'),
    text,
  )
}

function toDetails(text: string | undefined): string | undefined {
  if (!text) return undefined
  const redacted = redact(text).trim()
  if (!redacted) return undefined
  return redacted.length > DETAILS_MAX_LENGTH
    ? `${redacted.slice(0, DETAILS_MAX_LENGTH)}…`
    : redacted
}

/** Upstream text reaches the user, so it gets the same scrub as `details`. */
function safeMessage(text: string, fallback: string): string {
  const redacted = redact(text).trim()
  return redacted || fallback
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  return String(error)
}

/**
 * Recover the gateway's error code. `browseros-fetch` and `openrouter-fetch`
 * stash it on `data`, but the 12 direct-provider paths build their own
 * APICallError where `responseBody` is the only structured evidence.
 */
function gatewayCode(error: APICallError): string | undefined {
  const fromData = (error.data as { code?: unknown } | undefined)?.code
  if (typeof fromData === 'string' && fromData) return fromData

  if (!error.responseBody) return undefined
  try {
    const parsed = JSON.parse(error.responseBody)
    const code = parsed?.error?.code
    return typeof code === 'string' && code ? code : undefined
  } catch {
    return undefined
  }
}

function retryAfterSeconds(error: APICallError): number | undefined {
  const headers = error.responseHeaders
  if (!headers) return undefined
  const raw = headers['retry-after'] ?? headers['retry-after-ms']
  if (!raw) return undefined
  const value = Number(raw)
  if (!Number.isFinite(value) || value <= 0) return undefined
  return headers['retry-after-ms'] ? Math.ceil(value / 1000) : Math.ceil(value)
}

function fromApiCallError(
  error: APICallError,
  ctx: ChatErrorContext,
): ChatError {
  const base = {
    provider: ctx.provider,
    statusCode: error.statusCode,
    details: toDetails(error.message),
  }
  const code = gatewayCode(error)
  const isBrowserOs = ctx.provider === 'browseros'

  if (code === 'CREDITS_EXHAUSTED') {
    return {
      ...base,
      code: 'credits_exhausted',
      title: 'Daily limit reached',
      message: safeMessage(
        error.message,
        'You have used all your BrowserOS credits. They reset at midnight UTC.',
      ),
      retryable: false,
      docsUrl: USAGE_PAGE_URL,
    }
  }

  switch (error.statusCode) {
    case 401:
    case 403:
      return {
        ...base,
        code: 'auth_failed',
        title: 'Authentication failed',
        message: safeMessage(
          error.message,
          'The provider rejected your credentials. Check your API key in AI settings.',
        ),
        retryable: false,
      }
    case 429:
      return {
        ...base,
        code: 'rate_limited',
        title: 'Rate limited',
        message: safeMessage(
          error.message,
          'The provider is rate limiting requests. Wait a moment and try again.',
        ),
        retryable: true,
        docsUrl: isBrowserOs ? USAGE_DOCS_URL : undefined,
        retryAfterSeconds: retryAfterSeconds(error),
      }
    case 413:
      return {
        ...base,
        code: 'context_length',
        title: 'Conversation too long',
        message: safeMessage(
          error.message,
          'This conversation exceeded the model context window. Start a new chat or pick a model with a larger window.',
        ),
        retryable: false,
      }
    default:
      break
  }

  if (error.statusCode !== undefined && error.statusCode >= 500) {
    return {
      ...base,
      code: 'provider_unavailable',
      title: 'Provider unavailable',
      message: safeMessage(
        error.message,
        'The model provider returned a server error. This is usually temporary.',
      ),
      retryable: true,
    }
  }

  return {
    ...base,
    code: 'unknown',
    title: 'Something went wrong',
    message: safeMessage(error.message, 'The model request failed.'),
    retryable: error.isRetryable,
  }
}

/** Last-resort rungs for errors that carry no structured evidence at all. */
function fromMessage(message: string, ctx: ChatErrorContext): ChatError | null {
  const lower = message.toLowerCase()

  if (lower.includes('failed to fetch') || lower.includes('fetch failed')) {
    return {
      code: 'connection_failed',
      title: 'Connection failed',
      message:
        'Could not reach the model provider. Check your network connection and any local provider URL.',
      retryable: true,
      provider: ctx.provider,
      docsUrl: CONNECTION_DOCS_URL,
      details: toDetails(message),
    }
  }

  if (
    lower.includes('context length') ||
    lower.includes('context_length_exceeded') ||
    lower.includes('too many tokens') ||
    lower.includes('maximum context')
  ) {
    return {
      code: 'context_length',
      title: 'Conversation too long',
      message:
        'This conversation exceeded the model context window. Start a new chat or pick a model with a larger window.',
      retryable: false,
      provider: ctx.provider,
      details: toDetails(message),
    }
  }

  if (
    lower.includes('content filter') ||
    lower.includes('content_filter') ||
    lower.includes('content policy')
  ) {
    return {
      code: 'content_filter',
      title: 'Content was rejected',
      message:
        'The provider rejected the content being sent or generated. Try rephrasing and send it again.',
      retryable: false,
      provider: ctx.provider,
      details: toDetails(message),
    }
  }

  if (
    lower.includes('requires apikey') ||
    lower.includes('requires baseurl') ||
    lower.includes('requires oauth') ||
    lower.includes('not authenticated with') ||
    lower.includes('is required for')
  ) {
    return {
      code: 'provider_config',
      title: 'Provider not configured',
      message: safeMessage(message, 'This provider is not fully configured.'),
      retryable: false,
      provider: ctx.provider,
    }
  }

  return null
}

/** Maps any thrown value onto the envelope the client renders. */
export function toChatError(
  error: unknown,
  ctx: ChatErrorContext = {},
): ChatError {
  // The SDK wraps exhausted retries; the last failure is the informative one.
  if (RetryError.isInstance(error)) {
    return toChatError(error.lastError ?? error.errors.at(-1), ctx)
  }

  if (APICallError.isInstance(error)) {
    return fromApiCallError(error, ctx)
  }

  if (LoadAPIKeyError.isInstance(error) || NoSuchModelError.isInstance(error)) {
    return {
      code: 'provider_config',
      title: 'Provider not configured',
      message: safeMessage(
        error.message,
        'This provider is not fully configured.',
      ),
      retryable: false,
      provider: ctx.provider,
    }
  }

  if (InvalidPromptError.isInstance(error)) {
    return {
      code: 'context_length',
      title: 'Message could not be sent',
      message: safeMessage(error.message, 'This message could not be sent.'),
      retryable: false,
      provider: ctx.provider,
    }
  }

  const message = errorText(error)
  const matched = fromMessage(message, ctx)
  if (matched) return matched

  return {
    code: 'unknown',
    title: 'Something went wrong',
    message: safeMessage(message, 'An unexpected error occurred.'),
    retryable: true,
    provider: ctx.provider,
    details: AISDKError.isInstance(error) ? toDetails(error.name) : undefined,
  }
}

/** Serialized form for the `errorText` field of a UI message stream chunk. */
export function toChatErrorText(
  error: unknown,
  ctx: ChatErrorContext = {},
): string {
  return serializeChatError(toChatError(error, ctx))
}
