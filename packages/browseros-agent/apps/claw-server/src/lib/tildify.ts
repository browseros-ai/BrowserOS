/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home-directory-aware path compaction for anything the UI is going
 * to render as a file location. Replaces the runtime user's home dir
 * prefix with `~` so the caption stays short and the operator does
 * not see their username in the cockpit chrome. Non-home paths pass
 * through unchanged.
 */

import { homedir } from 'node:os'

/** Cache the resolved home dir to avoid re-reading env every call. */
const HOME = (() => {
  try {
    return homedir()
  } catch {
    return ''
  }
})()

export function tildifyHomePath(path: string | undefined): string | undefined {
  if (!path) return path
  if (!HOME) return path
  // The home dir may or may not have a trailing slash on the input;
  // match both `<HOME>` exact (unlikely for a config path) and
  // `<HOME>/...` prefix.
  if (path === HOME) return '~'
  if (path.startsWith(`${HOME}/`)) return `~${path.slice(HOME.length)}`
  return path
}
