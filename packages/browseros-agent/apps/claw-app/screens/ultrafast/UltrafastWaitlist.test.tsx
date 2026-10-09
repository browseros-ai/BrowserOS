import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

mock.module('@/components/ui/button', () => ({
  Button: (props: ComponentProps<'button'>) =>
    createElement('button', { type: 'button', ...props }),
}))

type ViewProps = { price: 10 | 20; joined: boolean; onJoin: () => void }
let UltrafastWaitlistView: FC<ViewProps>

beforeAll(async () => {
  UltrafastWaitlistView = (await import('./UltrafastWaitlist'))
    .UltrafastWaitlistView
})

describe('UltrafastWaitlistView', () => {
  it('shows the assigned monthly price and the join button', () => {
    const html = renderToStaticMarkup(
      createElement(UltrafastWaitlistView, {
        price: 20,
        joined: false,
        onJoin: () => {},
      }),
    )
    expect(html).toContain('$20')
    expect(html).toContain('/month')
    expect(html).toContain('Join the waitlist</button>')
  })

  it('confirms the signup instead of offering the button again', () => {
    const html = renderToStaticMarkup(
      createElement(UltrafastWaitlistView, {
        price: 10,
        joined: true,
        onJoin: () => {},
      }),
    )
    expect(html).toContain('$10')
    expect(html).toContain('on the list')
    expect(html).not.toContain('<button')
  })
})
