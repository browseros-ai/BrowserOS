/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MiniScreencast } from './MiniScreencast'

describe('MiniScreencast', () => {
  it('renders the placeholder globe + host when no screencast is supplied', () => {
    const html = renderToStaticMarkup(<MiniScreencast site="example.com" />)
    expect(html).toContain('example.com')
    expect(html).not.toContain('data:image/jpeg;base64,')
  })

  it('renders the JPEG when a screencast frame is supplied', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast
        site="example.com"
        screencast={{ jpegBase64: 'AAAA', capturedAt: 1 }}
      />,
    )
    expect(html).toContain('data:image/jpeg;base64,AAAA')
    expect(html).toContain('Live view of example.com')
  })

  it('falls back to placeholder when screencast is null', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast site="example.com" screencast={null} />,
    )
    expect(html).not.toContain('data:image/jpeg;base64,')
    expect(html).toContain('example.com')
  })

  it('shows the live dot when live=true', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast site="example.com" live />,
    )
    expect(html).toMatch(/animate-pulse-dot/)
  })

  it('does not show the live dot when live is false', () => {
    const html = renderToStaticMarkup(<MiniScreencast site="example.com" />)
    expect(html).not.toMatch(/animate-pulse-dot/)
  })
})
