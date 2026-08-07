/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  FOOTER_COPY,
  getOnboardingState,
  HERO_COPY,
  MANAGE_COPY,
  PANEL_COPY,
  STARTER_PROMPT,
} from './cockpit-onboarding.helpers'

describe('getOnboardingState', () => {
  it('returns first-run when no connection and no activity', () => {
    expect(
      getOnboardingState({ hasConnection: false, hasActivity: false }),
    ).toBe('first-run')
  })

  it('returns waiting when connection is installed but no activity yet', () => {
    expect(
      getOnboardingState({ hasConnection: true, hasActivity: false }),
    ).toBe('waiting')
  })

  it('returns ready as soon as any activity exists, regardless of connection state', () => {
    expect(
      getOnboardingState({ hasConnection: false, hasActivity: true }),
    ).toBe('ready')
    expect(getOnboardingState({ hasConnection: true, hasActivity: true })).toBe(
      'ready',
    )
  })
})

describe('cockpit onboarding copy', () => {
  // Snapshot-style tests that pin the shipped copy so accidental
  // voice drift is caught. Update these on purpose; every string
  // change here is a copy-pass decision, not a mechanical fix.
  it('hero copy carries the mental-model headline and a welcoming frame', () => {
    expect(HERO_COPY.eyebrow).toBe('WELCOME')
    expect(HERO_COPY.h1Prefix).toBe('You watch. Your agent')
    expect(HERO_COPY.h1Accent).toBe('works.')
    expect(HERO_COPY.subhead).toBe(
      'Your agents are wired in. Watch a quick demo, then hand your first task to any of them.',
    )
  })

  it('starter prompt is the streaming-price research task', () => {
    expect(STARTER_PROMPT).toBe(
      'Using BrowserOS neo, search for the current monthly prices of streaming services such as Netflix, Disney plus, Hulu, Max and Apple TV',
    )
  })

  it('panel status copy pins the heading and the three fixed-slot messages', () => {
    expect(PANEL_COPY.heading).toBe('Hand off your first task')
    expect(PANEL_COPY.tieBack).toBe('Then come back here to watch it run.')
    expect(PANEL_COPY.waiting).toContain('Waiting for your first run.')
    expect(PANEL_COPY.copied).toContain('Copied.')
  })

  it('manage-agents link points at the MCP screen', () => {
    expect(MANAGE_COPY.label).toBe('Manage agents')
    expect(MANAGE_COPY.href).toBe('/mcp')
  })

  it('docs footer link deep-links to the BrowserClaw section', () => {
    // Should NOT bare-root; readers arriving from the cockpit expect
    // BrowserClaw-specific docs (install / MCP / first-run), not the
    // BrowserOS index. Guard against accidental drift.
    expect(FOOTER_COPY.docsHref).toBe('https://docs.browseros.com/browserclaw')
    expect(FOOTER_COPY.docs).toBe('Read the docs')
  })
})
