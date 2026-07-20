/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { sessionBrowserTabPreviewUrl } from '@/modules/api/audit.hooks'
import { MiniScreencast } from './MiniScreencast'

describe('MiniScreencast', () => {
  it('renders the placeholder globe and host without a browser tab', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast site="No browser activity" sessionId="session-empty" />,
    )
    expect(html).toContain('No browser activity')
    expect(html).not.toContain('data:image/jpeg;base64,')
    expect(html).not.toContain('data-preview-url')
  })

  it('keys the canonical JPEG URL by session, browser tab, and capture time', () => {
    expect(
      sessionBrowserTabPreviewUrl(
        'session / one',
        7,
        123,
        'http://127.0.0.1:9200',
      ),
    ).toBe(
      'http://127.0.0.1:9200/api/v1/sessions/session%20%2F%20one/browser-tabs/7/preview?capturedAt=123',
    )
  })

  it('falls back to placeholder before a preview has been captured', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast
        site="example.com"
        sessionId="session-1"
        browserTabId={7}
      />,
    )
    expect(html).not.toContain('data:image/jpeg;base64,')
    expect(html).not.toContain('data-preview-url')
    expect(html).toContain('example.com')
  })

  it('shows the live dot only when live=true', () => {
    const liveHtml = renderToStaticMarkup(
      <MiniScreencast
        site="example.com"
        sessionId="session-1"
        browserTabId={7}
        live
      />,
    )
    const idleHtml = renderToStaticMarkup(
      <MiniScreencast
        site="example.com"
        sessionId="session-1"
        browserTabId={7}
      />,
    )
    expect(liveHtml).toMatch(/animate-pulse-dot/)
    expect(idleHtml).not.toMatch(/animate-pulse-dot/)
  })
})
