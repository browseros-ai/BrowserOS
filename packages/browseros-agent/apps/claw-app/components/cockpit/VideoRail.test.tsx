/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { VideoRail } from './VideoRail'

describe('VideoRail', () => {
  it('renders the fold header and, expanded by default, the video tiles as posters', () => {
    const html = renderToStaticMarkup(<VideoRail />)
    expect(html).toContain('Learn BrowserClaw')
    expect(html).toContain('aria-expanded="true"')
    // Expanded by default (no stored preference), so tiles render with their
    // play affordance and no iframe until the lightbox opens.
    expect(html).toContain('Can AI Agents Finally Automate the Web?')
    expect(html).toContain('Play: ')
    expect(html).not.toContain('youtube-nocookie.com/embed')
  })
})
