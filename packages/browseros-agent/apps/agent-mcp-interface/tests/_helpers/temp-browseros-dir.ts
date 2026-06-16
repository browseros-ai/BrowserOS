/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Test helper: redirects the interface package's storage root to a
 * fresh tmp directory so each test is isolated. The override is set
 * on the shared `env` object (read once at module load), then nudged
 * back to its prior value after the test.
 *
 * Use as a wrapper:
 *
 *   await withTempBrowserosDir(async () => {
 *     // body runs against an isolated <browserosDir>
 *   })
 */

import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { env } from '../../src/env'

export async function withTempBrowserosDir<T>(
  body: (dir: string) => Promise<T>,
): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), 'browseros-mcp-interface-'))
  const prior = env.browserosDirOverride
  env.browserosDirOverride = dir
  try {
    return await body(dir)
  } finally {
    env.browserosDirOverride = prior
    await rm(dir, { recursive: true, force: true })
  }
}
