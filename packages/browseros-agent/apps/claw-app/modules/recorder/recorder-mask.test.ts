import { describe, expect, it } from 'bun:test'
import { isSensitiveInput, maskRecorderInput } from './recorder-mask'

/** A minimal stand-in for the element rrweb hands maskInputFn: the attribute surface the classifier reads. */
function field(options: {
  tag?: string
  type?: string
  autocomplete?: string
}): HTMLElement {
  const attrs: Record<string, string> = {}
  if (options.type !== undefined) attrs.type = options.type
  if (options.autocomplete !== undefined)
    attrs.autocomplete = options.autocomplete
  return {
    tagName: options.tag ?? 'INPUT',
    type: options.type ?? '',
    getAttribute: (name: string) => attrs[name] ?? null,
  } as unknown as HTMLElement
}

describe('isSensitiveInput', () => {
  it('flags a password input', () => {
    expect(isSensitiveInput(field({ type: 'password' }))).toBe(true)
  })

  it('flags password and one-time-code autocomplete on a text field', () => {
    expect(
      isSensitiveInput(
        field({ type: 'text', autocomplete: 'current-password' }),
      ),
    ).toBe(true)
    expect(
      isSensitiveInput(
        field({ type: 'password', autocomplete: 'new-password' }),
      ),
    ).toBe(true)
    expect(
      isSensitiveInput(field({ type: 'text', autocomplete: 'one-time-code' })),
    ).toBe(true)
  })

  it('flags payment-card fields by their cc-* autocomplete token', () => {
    for (const token of ['cc-number', 'cc-csc', 'cc-exp', 'cc-name']) {
      expect(
        isSensitiveInput(field({ type: 'text', autocomplete: token })),
      ).toBe(true)
    }
    // A multi-token autocomplete still matches on the field-name token.
    expect(
      isSensitiveInput(
        field({
          type: 'text',
          autocomplete: 'section-blue shipping cc-number',
        }),
      ),
    ).toBe(true)
  })

  it('leaves ordinary fields unflagged', () => {
    expect(
      isSensitiveInput(field({ type: 'text', autocomplete: 'username' })),
    ).toBe(false)
    expect(
      isSensitiveInput(field({ type: 'email', autocomplete: 'email' })),
    ).toBe(false)
    expect(isSensitiveInput(field({ type: 'search' }))).toBe(false)
    expect(isSensitiveInput(field({ tag: 'TEXTAREA' }))).toBe(false)
    expect(isSensitiveInput(null)).toBe(false)
  })
})

describe('maskRecorderInput', () => {
  it('masks a sensitive value and preserves an ordinary one', () => {
    expect(maskRecorderInput('hunter2', field({ type: 'password' }))).toBe(
      '*******',
    )
    expect(
      maskRecorderInput(
        '4242424242424242',
        field({ type: 'text', autocomplete: 'cc-number' }),
      ),
    ).toBe('*'.repeat(16))
    expect(
      maskRecorderInput(
        'alice',
        field({ type: 'text', autocomplete: 'username' }),
      ),
    ).toBe('alice')
  })
})
