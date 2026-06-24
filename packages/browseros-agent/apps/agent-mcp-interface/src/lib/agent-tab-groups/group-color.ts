/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Deterministic agent slug to colour mapping for the v2 tab-group
 * system. The browser's `tab_groups` tool accepts one of nine named
 * colours; we hash the slug into one slot so two parallel sessions
 * under the same `clientInfo.name` always pick the same colour.
 *
 * The matching hex palette is exposed alongside so the cockpit
 * homepage card can show a left border in the same colour the
 * BrowserOS tab strip uses. The two ends sharing one function keeps
 * the colour story stable end-to-end.
 */

/** Colours the browser's `tab_groups` tool accepts. */
export const TAB_GROUP_COLORS = [
  'grey',
  'blue',
  'red',
  'yellow',
  'green',
  'pink',
  'purple',
  'cyan',
  'orange',
] as const

export type TabGroupColor = (typeof TAB_GROUP_COLORS)[number]

/**
 * Hex equivalents for each `TabGroupColor`. Used by the UI so the
 * card's left border visually mirrors the BrowserOS tab strip.
 * Picked to roughly match the system colours Chromium uses for tab
 * groups so the cross-surface mapping reads cleanly.
 */
export const TAB_GROUP_HEX: Record<TabGroupColor, string> = {
  grey: '#6B7280',
  blue: '#2F6FE0',
  red: '#DC2626',
  yellow: '#F59E0B',
  green: '#10A37F',
  pink: '#DB2777',
  purple: '#7A5AF8',
  cyan: '#0EA5E9',
  orange: '#F26B2A',
}

/**
 * FNV-1a 32-bit hash. Same algorithm used by `fallbackSlugForSession`
 * in `mcp-session/identity.ts`, so colour selection has the same
 * distribution as the synthetic-slug fallback already does.
 */
function fnv1a(input: string): number {
  let hash = 0x811c9dc5
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i)
    hash =
      (hash +
        ((hash << 1) +
          (hash << 4) +
          (hash << 7) +
          (hash << 8) +
          (hash << 24))) >>>
      0
  }
  return hash
}

/**
 * Picks a `TabGroupColor` for the given agent slug. Stable across
 * processes and across UI / server boundaries.
 */
export function colorForSlug(slug: string): TabGroupColor {
  const idx = fnv1a(slug) % TAB_GROUP_COLORS.length
  return TAB_GROUP_COLORS[idx] ?? 'grey'
}

/** Convenience: hex for the colour the agent group will use. */
export function hexForSlug(slug: string): string {
  return TAB_GROUP_HEX[colorForSlug(slug)]
}
