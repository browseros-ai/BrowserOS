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
  PRIMARY_ACTION_COPY,
  STARTER_PROMPT,
  STEP_COPY,
  WAITING_COPY,
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
  it('hero copy carries the mental-model headline and set-up-once frame', () => {
    expect(HERO_COPY.eyebrow).toBe('GET STARTED')
    expect(HERO_COPY.h1Prefix).toBe('You watch. Your agent')
    expect(HERO_COPY.h1Accent).toBe('works.')
    expect(HERO_COPY.subhead).toBe(
      'Set up BrowserClaw once. Tell your AI to use it. Everything lands here.',
    )
  })

  it('starter prompt is the SFO to NYC flight-booking task', () => {
    expect(STARTER_PROMPT).toBe(
      'Use BrowserClaw. Book me the cheapest morning flight from SFO to NYC next Friday.',
    )
  })

  it('waiting-banner copy differentiates connected-no-activity from prompt-copied', () => {
    expect(WAITING_COPY.connectedNoActivity).toContain(
      'Waiting for your first run.',
    )
    expect(WAITING_COPY.promptCopied).toContain('Prompt copied.')
  })

  it('primary action labels flip between active and done states', () => {
    expect(PRIMARY_ACTION_COPY.install.activeLabel).toBe('Set up MCP endpoint')
    expect(PRIMARY_ACTION_COPY.install.doneLabel).toBe('View MCP endpoint')
    expect(PRIMARY_ACTION_COPY.install.href).toBe('/mcp')
  })

  it('docs footer link deep-links to the BrowserClaw section', () => {
    // Should NOT bare-root; readers arriving from the cockpit expect
    // BrowserClaw-specific docs (install / MCP / first-run), not the
    // BrowserOS index. Guard against accidental drift.
    expect(FOOTER_COPY.docsHref).toBe('https://docs.browseros.com/browserclaw/')
    expect(FOOTER_COPY.docs).toBe('Read the docs')
  })

  it('step titles are single short sentences ending in a period', () => {
    for (const title of [
      STEP_COPY.install.activeTitle,
      STEP_COPY.install.doneTitle,
      STEP_COPY.ask.title,
      STEP_COPY.watch.title,
    ]) {
      expect(title.endsWith('.')).toBe(true)
    }
  })
})
