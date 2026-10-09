import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

mock.module('@/components/ui/button', () => ({
  Button: (props: ComponentProps<'button'>) =>
    createElement('button', { type: 'button', ...props }),
}))

mock.module('@/components/ui/input', () => ({
  Input: (props: ComponentProps<'input'>) => createElement('input', props),
}))

type ViewProps = {
  price: 10 | 20
  joined: boolean
  canJoin: boolean
  onJoin: (email: string) => void
}
let UltrafastWaitlistView: FC<ViewProps>
let normalizeEmail: (value: string) => string | null

beforeAll(async () => {
  const mod = await import('./UltrafastWaitlist')
  UltrafastWaitlistView = mod.UltrafastWaitlistView
  normalizeEmail = mod.normalizeEmail
})

function render(props: Partial<ViewProps>) {
  return renderToStaticMarkup(
    createElement(UltrafastWaitlistView, {
      price: 20,
      joined: false,
      canJoin: true,
      onJoin: () => {},
      ...props,
    }),
  )
}

describe('UltrafastWaitlistView', () => {
  it('shows the assigned monthly price and an email signup form', () => {
    const html = render({ price: 20 })
    expect(html).toContain('$20')
    expect(html).toContain('/month')
    expect(html).toContain('type="email"')
    expect(html).toContain('Join the waitlist</button>')
    expect(html).not.toContain('usage analytics, which is off')
  })

  it('confirms the signup instead of offering the form again', () => {
    const html = render({ price: 10, joined: true })
    expect(html).toContain('$10')
    expect(html).toContain('on the list')
    expect(html).not.toContain('<form')
  })

  it('disables signup and explains why when analytics is off', () => {
    const html = render({ canJoin: false })
    expect(html).toContain('disabled=""')
    expect(html).toContain('usage analytics, which is off')
  })
})

describe('normalizeEmail', () => {
  it('trims and lowercases a valid email', () => {
    expect(normalizeEmail('  Ada@Example.COM ')).toBe('ada@example.com')
  })

  it('rejects values that are not emails', () => {
    expect(normalizeEmail('')).toBeNull()
    expect(normalizeEmail('ada')).toBeNull()
    expect(normalizeEmail('ada@example')).toBeNull()
    expect(normalizeEmail('a da@example.com')).toBeNull()
  })
})
