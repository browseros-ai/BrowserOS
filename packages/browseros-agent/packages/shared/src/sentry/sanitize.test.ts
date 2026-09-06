import { expect, test } from 'bun:test'
import { sanitizeEvent } from './sanitize'

test('redacts all custom header values from captured configuration', () => {
  const event: { extra: Record<string, unknown> } = {
    extra: {
      provider: 'openai-compatible',
      headers: { 'x-private-key': 'secret' },
    },
  }
  expect(sanitizeEvent(event).extra).toEqual({
    provider: 'openai-compatible',
    headers: '[REDACTED]',
  })
})
