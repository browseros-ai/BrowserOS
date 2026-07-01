/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { AutoHideTabs } from './tabs-auto-hide'

describe('AutoHideTabs', () => {
  it('renders nothing when given an empty items array', () => {
    const html = renderToStaticMarkup(<AutoHideTabs items={[]} />)
    expect(html).toBe('')
  })

  it('renders the single item content without any tab triggers', () => {
    const html = renderToStaticMarkup(
      <AutoHideTabs
        items={[{ id: 'only', label: 'Only', content: <p>only content</p> }]}
      />,
    )
    // Content is present.
    expect(html).toContain('only content')
    // But no tablist / tab markers should exist.
    expect(html).not.toContain('data-slot="tabs-list"')
    expect(html).not.toContain('data-slot="tabs-trigger"')
    // And the trigger label itself is absent (single-item mode skips the bar).
    expect(html).not.toContain('Only')
  })

  it('renders full tabs UI when two or more items are given', () => {
    const html = renderToStaticMarkup(
      <AutoHideTabs
        items={[
          { id: 'a', label: 'Alpha', content: <p>alpha content</p> },
          { id: 'b', label: 'Bravo', content: <p>bravo content</p> },
        ]}
      />,
    )
    expect(html).toContain('data-slot="tabs-list"')
    expect(html).toContain('Alpha')
    expect(html).toContain('Bravo')
    // Both content panels render (the primitive keeps them mounted for
    // fast switching; only the selected one is visible via CSS).
    expect(html).toContain('alpha content')
  })
})
