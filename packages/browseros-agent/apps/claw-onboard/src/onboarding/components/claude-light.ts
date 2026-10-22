/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CSSProperties } from 'react'

/**
 * The Claude desktop app's default light theme, sampled from its Code mode
 * (round 8), so the drawn Claude window reads as the app people have. It is
 * set as CSS variables on that window's root and read with `bg-(--claude-…)`
 * classes.
 * A copy of claw-app's components/cockpit/onboarding-demo/claude-light.ts:
 * claw-onboard ships as Chromium WebUI resources and cannot import from
 * claw-app, so keep the two in sync.
 */
export const CLAUDE_LIGHT = {
  '--claude-sidebar': '#FBFBF9',
  '--claude-main': '#FCFCFB',
  '--claude-selected': '#EDECE9',
  '--claude-bubble': '#F0EFED',
  '--claude-toggle': '#EFEFED',
  '--claude-border': '#E3E3E1',
  '--claude-composer-border': '#D9D8D5',
  '--claude-ink': '#0B0B0B',
  '--claude-text': '#52514F',
  '--claude-muted': '#8A8781',
  '--claude-orange': '#D97757',
} as CSSProperties
