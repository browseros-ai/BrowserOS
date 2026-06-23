/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Resolves the on-disk directory the interface server reads + writes
 * under. Path logic intentionally mirrors `apps/server`'s own
 * resolver so a user pointing both servers at the same machine sees
 * consistent file locations; env reads stay scoped per package.
 *
 * Order of preference:
 *   1. `BROWSEROS_DIR` env override (read once via `env.ts`).
 *   2. `<homedir>/.browseros-dev` when `NODE_ENV === 'development'`.
 *   3. `<homedir>/.browseros` otherwise.
 *
 * The interface package writes everything under
 * `<browserosDir>/mcp-interface/` so other BrowserOS components
 * (server, CLI) keep their own subtrees untouched.
 */

import { homedir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import { env } from '../env'

const INTERFACE_SUBDIR = 'mcp-interface'

export function getBrowserosDir(): string {
  if (env.browserosDirOverride) return env.browserosDirOverride
  const dirName = env.isDevelopment
    ? PATHS.DEV_BROWSEROS_DIR_NAME
    : PATHS.BROWSEROS_DIR_NAME
  return join(homedir(), dirName)
}

/** `<browserosDir>/mcp-interface`, the root for this package's files. */
export function getInterfaceDir(): string {
  return join(getBrowserosDir(), INTERFACE_SUBDIR)
}

/** Convenience: any relative path resolved against the interface root. */
export function resolveInterfacePath(...segments: string[]): string {
  return join(getInterfaceDir(), ...segments)
}
