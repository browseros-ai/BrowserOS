/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * BrowserClaw keeps its own state tree separate from BrowserOS server
 * state, while still using the same dev/prod split convention.
 */

import { homedir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import { env } from '../env'

/** Returns the BrowserClaw state root used by logs, DB, replays, and agent metadata. */
export function getClawServerDir(): string {
  if (env.browserClawDirOverride) return env.browserClawDirOverride
  const dirName = env.isDevelopment
    ? PATHS.DEV_BROWSERCLAW_DIR_NAME
    : PATHS.BROWSERCLAW_DIR_NAME
  return join(homedir(), dirName)
}

/** Resolves a relative path against the BrowserClaw state root. */
export function resolveClawServerPath(...segments: string[]): string {
  return join(getClawServerDir(), ...segments)
}
