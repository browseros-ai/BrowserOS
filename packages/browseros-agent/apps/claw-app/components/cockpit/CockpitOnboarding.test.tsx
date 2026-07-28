/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { CockpitOnboarding } from './CockpitOnboarding'

function render(
  state: 'first-run' | 'waiting',
  connectedHarnesses: readonly string[] = [],
): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <CockpitOnboarding
        state={state}
        connectedHarnesses={connectedHarnesses}
      />
    </MemoryRouter>,
  )
}

describe('CockpitOnboarding', () => {
  it('first-run: hero, a single focused how-to video, install CTA, and prompt tile render', () => {
    const html = render('first-run')
    expect(html).toContain('You watch. Your agent')
    expect(html).toContain('works.')
    // The feature video renders as a poster with a play affordance; no
    // iframe or self-hosted source is present until the lightbox opens.
    expect(html).toContain('Can AI Agents Finally Automate the Web?')
    expect(html).toContain('Play: ')
    expect(html).not.toContain('youtube-nocookie.com/embed')
    expect(html).toContain('Set up MCP endpoint')
    expect(html).toContain('Paste this into Claude Code, Cursor, or Codex.')
    expect(html).toContain(
      'Use BrowserClaw. Book me the cheapest morning flight',
    )
  })

  it('first-run: retires the single setup demo and the numbered step strip', () => {
    const html = render('first-run')
    expect(html).not.toContain('first-run-demo.mp4')
    expect(html).not.toContain('Install BrowserClaw as an MCP.')
    expect(html).not.toContain('Watch it here.')
  })

  it('first-run: renders neither the waiting banner nor a connected-agents line', () => {
    const html = render('first-run')
    expect(html).not.toContain(
      'Waiting for your first run. Come back here as soon',
    )
    expect(html).not.toContain('connected')
  })

  it('waiting: banner renders, install CTA relabels to View, and connected agents list', () => {
    const html = render('waiting', ['Claude Code', 'Cursor'])
    expect(html).toContain('Waiting for your first run. Come back here as soon')
    expect(html).toContain('View MCP endpoint')
    expect(html).not.toContain('Set up MCP endpoint')
    expect(html).toContain('Claude Code, Cursor')
    expect(html).toContain('connected')
  })

  it('waiting: retains the starter prompt tile so the reader can still copy', () => {
    const html = render('waiting', ['Claude Code'])
    expect(html).toContain(
      'Use BrowserClaw. Book me the cheapest morning flight',
    )
  })

  it('renders the docs link in both states with no refresh affordance', () => {
    for (const state of ['first-run', 'waiting'] as const) {
      const html = render(state)
      expect(html).toContain('https://docs.browseros.com/')
      expect(html).not.toContain('Refresh the page.')
      expect(html).not.toContain('Already set up?')
    }
  })
})
