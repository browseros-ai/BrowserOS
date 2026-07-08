/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { FirstRunVideo } from './FirstRunVideo'

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')

afterEach(() => {
  if (originalWindow) {
    Object.defineProperty(globalThis, 'window', originalWindow)
    return
  }
  Reflect.deleteProperty(globalThis, 'window')
})

function renderWithReducedMotion(matches: boolean): string {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      matchMedia: (query: string) =>
        ({
          matches,
          media: query,
          onchange: null,
          addEventListener: () => {},
          removeEventListener: () => {},
          addListener: () => {},
          removeListener: () => {},
          dispatchEvent: () => false,
        }) as MediaQueryList,
    } satisfies Partial<Window>,
  })
  return renderToStaticMarkup(<FirstRunVideo />)
}

describe('FirstRunVideo', () => {
  it('renders a playable video without autoplay when reduced motion is preferred', () => {
    const html = renderWithReducedMotion(true)

    expect(html).toContain('<video')
    expect(html).toContain('first-run-demo.mp4')
    expect(html).toContain('first-run-demo-poster.png')
    expect(html).toContain('controls=""')
    expect(html).not.toContain('autoPlay=""')
    expect(html).not.toContain('loop=""')
    expect(html).not.toContain('pointer-events-none')
    expect(html).not.toContain('<img')
  })

  it('preserves muted autoplay looping when reduced motion is not preferred', () => {
    const html = renderWithReducedMotion(false)

    expect(html).toContain('<video')
    expect(html).toContain('autoPlay=""')
    expect(html).toContain('muted=""')
    expect(html).toContain('loop=""')
    expect(html).toContain('playsInline=""')
    expect(html).not.toContain('controls=""')
    expect(html).toContain('pointer-events-none')
  })
})
