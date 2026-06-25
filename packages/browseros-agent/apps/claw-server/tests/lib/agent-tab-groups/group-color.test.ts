import { describe, expect, it } from 'bun:test'
import {
  colorForSlug,
  hexForSlug,
  TAB_GROUP_COLORS,
  TAB_GROUP_HEX,
} from '../../../src/lib/agent-tab-groups/group-color'

describe('colorForSlug', () => {
  it('returns the same colour for the same slug across calls', () => {
    expect(colorForSlug('claude-code')).toBe(colorForSlug('claude-code'))
    expect(colorForSlug('cursor-bot')).toBe(colorForSlug('cursor-bot'))
  })

  it('returns one of the nine allowed tab_groups colours', () => {
    for (const slug of ['claude-code', 'cursor', 'codex', 'zed', 'vscode']) {
      expect(TAB_GROUP_COLORS).toContain(colorForSlug(slug))
    }
  })

  it('distributes common harness names across several colours, not all the same', () => {
    const slugs = [
      'claude-code',
      'cursor',
      'codex',
      'zed',
      'vscode',
      'claude-desktop',
      'gemini',
    ]
    const colours = new Set(slugs.map(colorForSlug))
    expect(colours.size).toBeGreaterThanOrEqual(3)
  })

  it('returns a defined colour even for empty / unicode-degenerate slugs', () => {
    expect(TAB_GROUP_COLORS).toContain(colorForSlug(''))
    expect(TAB_GROUP_COLORS).toContain(colorForSlug('unknown-abc123'))
  })
})

describe('hexForSlug', () => {
  it('returns the hex matching the colour pick', () => {
    const slug = 'claude-code'
    expect(hexForSlug(slug)).toBe(TAB_GROUP_HEX[colorForSlug(slug)])
  })

  it('looks like a hex colour', () => {
    expect(hexForSlug('zed')).toMatch(/^#[0-9A-F]{6}$/)
  })
})
