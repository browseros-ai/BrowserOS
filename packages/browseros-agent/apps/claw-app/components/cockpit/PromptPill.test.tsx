/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { PromptPill } from './PromptPill'

describe('PromptPill', () => {
  it('renders the prompt verbatim as one copy button', () => {
    const html = renderToStaticMarkup(
      <PromptPill prompt="Read me today’s top news." />,
    )
    expect(html).toContain('Read me today’s top news.')
    expect(html).toContain('aria-label="Copy: Read me today’s top news."')
    expect(html.match(/<button/g)).toHaveLength(1)
    expect(html).not.toContain('Copied')
  })
})
