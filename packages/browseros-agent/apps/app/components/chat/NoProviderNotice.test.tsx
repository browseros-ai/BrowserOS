/**
 * Pins the copy each audience sees. A brand-new user and someone whose hosted
 * provider was retired both have nothing connected, and telling the second one
 * to "get started" would not explain why a working chat stopped working.
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { NoProviderNotice } from './NoProviderNotice'

describe('NoProviderNotice', () => {
  it('invites a first-time user to connect something', () => {
    const html = renderToStaticMarkup(<NoProviderNotice />)

    expect(html).toContain('Connect a provider to start chatting')
    expect(html).toContain('Connect a provider')
    expect(html).not.toContain('no longer ships')
  })

  it('explains the retirement to someone who was on the built-in model', () => {
    const html = renderToStaticMarkup(<NoProviderNotice retired />)

    expect(html).toContain('BrowserOS no longer ships a built-in model')
    expect(html).toContain('Your conversations and settings are unchanged.')
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
    for (const props of [{}, { retired: true }, { blocked: true }]) {
      const html = renderToStaticMarkup(<NoProviderNotice {...props} />)
      expect(html).toContain('/app.html#/onboarding/ai')
    }
  })
})
