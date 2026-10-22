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

describe('CockpitOnboarding', () => {
  it('lists the connected apps with their marks and the restart hint', () => {
    const html = render(['Claude Code', 'Codex'])
    expect(html).toContain('Your AI apps are connected.')
    expect(html).toContain('Claude Code')
    expect(html).toContain('Codex')
    expect(html.match(/aria-label="connected"/g)).toHaveLength(2)
    expect(html).toContain('If one is open, restart it once.')
    expect(html).not.toContain('I didn’t find an AI app yet.')
  })

  it('shows every connected app, never a +N overflow', () => {
    const harnesses: Harness[] = [
      'Claude Code',
      'Codex',
      'Cursor',
      'OpenCode',
      'Antigravity',
      'VS Code',
      'Zed',
    ]
    const html = render(harnesses)
    for (const harness of harnesses) expect(html).toContain(harness)
    expect(html).not.toContain('+3')
  })

  it('with no app found, offers Get Claude and leads with Cowork', () => {
    const html = render()
    expect(html).toContain('I didn’t find an AI app yet.')
    expect(html).toContain('href="https://claude.ai/download"')
    expect(html).not.toContain('Your AI apps are connected.')
    expect(html).toMatch(/bg-accent text-accent-foreground[^"]*">Set up Cowork/)
  })

  it('keeps the Cowork step in both states', () => {
    for (const harnesses of [[], ['Claude Code']] as Harness[][]) {
      const html = render(harnesses)
      expect(html).toContain('Using Claude Cowork? It needs one last step.')
      expect(html).toContain('Set up Cowork')
    }
  })

  it('replaces the video with the demo and two copyable prompts', () => {
    const html = render(['Claude Code'])
    expect(html).toContain('Watch me do one.')
    expect(html).not.toContain('<video')
    expect(html).toContain('aria-label="Pause the demo"')
    expect(html).toContain('aria-label="Replay the demo"')
    expect(html).toContain('Works the same in Codex and Terminal.')
    expect(html).toContain('Try one')
    expect(html).toContain(
      'aria-label="Copy: Find the cheapest flight to New York next Friday."',
    )
    expect(html).toContain('aria-label="Copy: Read me today’s top news."')
    expect(html).toContain(
      'Paste one into Claude Code or Codex, then watch it here.',
    )
  })

  it('reads without jargon', () => {
    const text = render(['Claude Code']).replace(/<[^>]+>/g, ' ')
    expect(text).not.toMatch(/\b(MCP|server|endpoint|config)\b/i)
  })
})
