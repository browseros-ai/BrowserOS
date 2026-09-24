/** Pins the copy shown when nothing is connected, resting and after a refused send. */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { NoProviderNotice } from './NoProviderNotice'

describe('NoProviderNotice', () => {
  it('invites the reader to connect something', () => {
    const html = renderToStaticMarkup(<NoProviderNotice />)

    expect(html).toContain('Connect a provider to start chatting')
    expect(html).toContain('Connect a provider')
  })

  it('answers a refused send and says the message was kept', () => {
    const html = renderToStaticMarkup(<NoProviderNotice blocked />)

    expect(html).toContain('Nothing to send this to yet')
    expect(html).toContain('Your message is still here.')
    // Announced, because it is a response to something the user just did.
    expect(html).toContain('role="alert"')
  })

  it('is a status rather than an alert while it is only resting', () => {
    const html = renderToStaticMarkup(<NoProviderNotice />)

    expect(html).toContain('role="status"')
    expect(html).not.toContain('role="alert"')
  })

  it('always offers the way out', () => {
    for (const props of [{}, { blocked: true }]) {
      const html = renderToStaticMarkup(<NoProviderNotice {...props} />)
      expect(html).toContain('/app.html#/onboarding/ai')
    }
  })
})
