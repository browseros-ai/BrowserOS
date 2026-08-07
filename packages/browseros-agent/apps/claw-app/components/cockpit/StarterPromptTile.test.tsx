/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { StarterPromptTile } from './StarterPromptTile'

describe('StarterPromptTile', () => {
  it('renders the prompt verbatim and a Copy button', () => {
    const html = renderToStaticMarkup(
      <StarterPromptTile prompt="Use BrowserClaw. Do the thing." />,
    )
    expect(html).toContain('Use BrowserClaw. Do the thing.')
    expect(html).toContain('Copy')
  })
})
