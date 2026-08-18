import { beforeAll, describe, expect, it, mock } from 'bun:test'
import {
  type ChatError as ChatErrorEnvelope,
  serializeChatError,
} from '@browseros/shared/schemas/chat-error'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

let ChatError: FC<{
  error: Error
  onRetry?: () => void
  providerType?: string
}>

beforeAll(async () => {
  ChatError = (await import('./ChatError')).ChatError
})

function renderError(error: Error, providerType = 'browseros') {
  return renderToStaticMarkup(
    createElement(ChatError, {
      error,
      onRetry: () => {},
      providerType,
    }),
  )
}

function envelopeError(overrides: Partial<ChatErrorEnvelope> = {}): Error {
  return new Error(
    serializeChatError({
      code: 'unknown',
      title: 'Something went wrong',
      message: 'The model request failed.',
      retryable: true,
      ...overrides,
    }),
  )
}

describe('ChatError legacy string handling', () => {
  it('shows retry for connection errors', () => {
    const html = renderError(new Error('Failed to fetch'))

    expect(html).toContain('Try again')
  })

  it('hides retry for credits-exhausted errors', () => {
    const html = renderError(new Error('CREDITS_EXHAUSTED'))

    expect(html).toContain('Daily credits exhausted')
    expect(html).not.toContain('Try again')
  })

  it('hides retry for BrowserOS daily-limit errors', () => {
    const html = renderError(
      new Error('BrowserOS LLM daily limit reached for today'),
    )

    expect(html).toContain('Add your own API key')
    expect(html).not.toContain('Try again')
  })
})

describe('ChatError envelope handling', () => {
  it('renders the real reason for credit exhaustion and hides retry', () => {
    const html = renderError(
      envelopeError({
        code: 'credits_exhausted',
        title: 'Daily limit reached',
        message: 'You have used all your BrowserOS credits.',
        retryable: false,
        provider: 'browseros',
        docsUrl: '/app.html#/settings/usage',
      }),
    )

    expect(html).toContain('Daily limit reached')
    expect(html).toContain('You have used all your BrowserOS credits.')
    expect(html).toContain('View Usage &amp; Billing')
    expect(html).not.toContain('Try again')
  })

  it('surfaces auth failures with the provider status and a settings link', () => {
    const html = renderError(
      envelopeError({
        code: 'auth_failed',
        title: 'Authentication failed',
        message: 'Incorrect API key provided.',
        retryable: false,
        statusCode: 401,
        provider: 'anthropic',
      }),
      'anthropic',
    )

    expect(html).toContain('Authentication failed')
    expect(html).toContain('Incorrect API key provided.')
    expect(html).toContain('Open AI settings')
    expect(html).not.toContain('Try again')
  })

  it('offers retry for a transient provider outage', () => {
    const html = renderError(
      envelopeError({
        code: 'provider_unavailable',
        title: 'Provider unavailable',
        message: 'The provider returned a server error.',
        retryable: true,
        statusCode: 503,
      }),
    )

    expect(html).toContain('Provider unavailable')
    expect(html).toContain('Try again')
  })

  it('offers a details disclosure when raw upstream text is present', () => {
    const html = renderError(
      envelopeError({
        message: 'The model request failed.',
        details: 'upstream said: connection reset by peer',
      }),
    )

    expect(html).toContain('Show details')
  })

  it('omits the disclosure when there is no extra detail', () => {
    const html = renderError(envelopeError({ details: undefined }))

    expect(html).not.toContain('Show details')
  })

  it('falls back to the server-supplied title for an unknown code', () => {
    const html = renderError(
      envelopeError({
        code: 'unknown',
        title: 'Something went wrong',
        message: 'something exotic broke',
      }),
    )

    expect(html).toContain('something exotic broke')
  })
})
