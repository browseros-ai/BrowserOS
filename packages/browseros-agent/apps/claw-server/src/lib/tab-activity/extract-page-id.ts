/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pulls the `page` argument out of a browser-tool dispatch so the
 * cockpit's tab-activity registry can attribute the call to a tab.
 * Tools without a `page` parameter (`tab_groups`, `windows`, `run`)
 * always yield null. Tools that accept it optionally (`tabs` action
 * variants like `list` vs `close`) yield null when the caller omits
 * it. Non-integer / non-positive values are rejected to keep the
 * registry from holding garbage keys.
 */

/**
 * Tools that accept a `page` argument (either required or optional).
 * Reused by `register.ts`'s cross-agent page guard so both the
 * tab-activity registry attribution AND the isolation guard agree
 * on which tools can be page-targeted. Adding a new tool that
 * takes a page id belongs here.
 */
export const TOOLS_WITH_PAGE: ReadonlySet<string> = new Set([
  'act',
  'diff',
  'download',
  'evaluate',
  'grep',
  'navigate',
  'pdf',
  'read',
  'screenshot',
  'snapshot',
  'tabs',
  'upload',
  'wait',
])

export function extractPageId(
  toolName: string,
  rawArgs: unknown,
): number | null {
  if (!TOOLS_WITH_PAGE.has(toolName)) return null
  if (!rawArgs || typeof rawArgs !== 'object') return null
  const page = (rawArgs as { page?: unknown }).page
  if (typeof page !== 'number') return null
  if (!Number.isInteger(page) || page < 1) return null
  return page
}
