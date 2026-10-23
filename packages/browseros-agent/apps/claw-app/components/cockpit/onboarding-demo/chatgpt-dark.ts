/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CSSProperties } from 'react'

/**
 * The ChatGPT desktop app's dark theme, sampled from the owner's screenshot
 * of its Work mode (round 10), where Codex runs, as Paper uses it. Set as CSS variables on the
 * drawn ChatGPT window's root, like `CLAUDE_LIGHT` on Claude's.
 */
export const CHATGPT_DARK = {
  '--chatgpt-sidebar': '#2C2D39',
  '--chatgpt-main': '#292A36',
  '--chatgpt-selected': '#3B3B46',
  '--chatgpt-toggle': '#363742',
  '--chatgpt-toggle-on': '#393A45',
  '--chatgpt-composer': '#40414B',
  '--chatgpt-border': '#42434C',
  '--chatgpt-ink': '#F8F8F2',
  '--chatgpt-text': '#DADAD6',
  /** The unselected toggle half. */
  '--chatgpt-soft': '#D1D1CF',
  '--chatgpt-muted': '#8C8D90',
  /** The composer placeholder. */
  '--chatgpt-muted-2': '#A2A2A4',
} as CSSProperties
