/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { Harness } from '@/components/harness/harness.types'
import { CockpitOnboarding } from './CockpitOnboarding'

function render(connectedHarnesses: readonly Harness[] = []): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <CockpitOnboarding connectedHarnesses={connectedHarnesses} />
    </MemoryRouter>,
  )
}

const SEVEN: Harness[] = [
  'Claude Code',
  'Codex',
  'Cursor',
  'OpenCode',
  'Antigravity',
  'VS Code',
  'Zed',
]

describe('CockpitOnboarding', () => {
  it('leads with neo’s one line, not the cockpit headline or bubbles', () => {
    const html = render(['Claude Code', 'Codex'])
    expect(html).toContain('You’re all set.')
    expect(html).toContain('>Watch me</em>')
    expect(html).toContain('do one.')
    // The demo's drawn new tab still says it; the page itself does not.
    expect(html).not.toMatch(/<h1[^>]*>[^<]*What are your agents/)
    expect(html).not.toContain('Your AI apps are connected.')
  })

  it('shows the connected apps as marks with Connected and the restart hint', () => {
    const html = render(['Claude Code', 'Codex'])
    expect(html).toContain('aria-label="Connected apps: Claude Code, Codex"')
    expect(html).toContain('aria-expanded="false"')
    expect(html).toContain('>Connected</span>')
    expect(html).toContain('If one is open, restart it once.')
    expect(html).not.toMatch(/>\+\d/)
  })

  it('caps the marks at three and collapses the rest into +N', () => {
    const html = render(SEVEN)
    expect(html).toContain('>+4</span>')
    expect(html).toContain(`aria-label="Connected apps: ${SEVEN.join(', ')}"`)
  })

  it('with no app found, says so and makes Get Claude the one primary action', () => {
    const html = render()
    expect(html).toContain('Almost there.')
    expect(html).toContain('I didn’t find an AI app yet.')
    expect(html).toContain('href="https://claude.ai/download"')
    expect(html).not.toContain('Connected apps')
  })

  it('keeps the quiet Cowork link in both states', () => {
    for (const harnesses of [[], ['Claude Code']] as Harness[][]) {
      const html = render(harnesses)
      expect(html).toContain('Using Claude Cowork?')
      expect(html).toContain('Set it up')
    }
  })

  it('shows the demo and the two copyable prompts', () => {
    const html = render(['Claude Code'])
    expect(html).not.toContain('<video')
    expect(html).toContain('aria-label="Pause the demo"')
    expect(html).toContain('Try one')
    expect(html).toContain(
      'aria-label="Copy: Find the cheapest flight to New York next Friday."',
    )
    expect(html).toContain('aria-label="Copy: Read me today’s top news."')
    expect(html).toContain(
      'Paste one into Claude or ChatGPT, then watch it here.',
    )
  })

  it('reads without jargon', () => {
    const text = render(SEVEN).replace(/<[^>]+>/g, ' ')
    expect(text).not.toMatch(/\b(MCP|server|endpoint|config)\b/i)
  })
})
