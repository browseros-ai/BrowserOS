/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { CockpitOnboarding } from './CockpitOnboarding'

function render(state: 'first-run' | 'waiting'): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <CockpitOnboarding state={state} />
    </MemoryRouter>,
  )
}

describe('CockpitOnboarding', () => {
  it('first-run: hero, motion demo, single install CTA, prompt tile, and reminder strip render', () => {
    const html = render('first-run')
    expect(html).toContain('You watch. Your agent')
    expect(html).toContain('works.')
    expect(html).toContain('first-run-demo.mp4')
    expect(html).toContain('Set up MCP endpoint')
    expect(html).toContain('Paste this into Claude Code, Cursor, or Codex.')
    expect(html).toContain(
      'Use BrowserClaw. Book me the cheapest morning flight',
    )
    expect(html).toContain('Install BrowserClaw as an MCP.')
    expect(html).toContain('Prompt your agent.')
    expect(html).toContain('Watch it here.')
  })

  it('first-run: renders only ONE primary CTA button in the action row', () => {
    // The Copy starter prompt button was retired; the primary action
    // row now holds just the MCP install navigation link.
    const html = render('first-run')
    expect(html).not.toContain('Copy starter prompt')
  })

  it('first-run: does NOT render the waiting banner before any signal', () => {
    const html = render('first-run')
    expect(html).not.toContain(
      'Waiting for your first run. Come back here as soon',
    )
  })

  it('waiting: banner renders, install CTA relabels to View, step 01 marks done, step 02 goes active', () => {
    const html = render('waiting')
    expect(html).toContain('Waiting for your first run. Come back here as soon')
    expect(html).toContain('View MCP endpoint')
    expect(html).not.toContain('Set up MCP endpoint')
    expect(html).toContain('MCP installed.')
    expect(html).not.toContain('Install BrowserClaw as an MCP.')
  })

  it('waiting: retains the starter prompt tile so the reader can still copy', () => {
    const html = render('waiting')
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
