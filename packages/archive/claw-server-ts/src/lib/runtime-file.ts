/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Atomic writer for `<CONFIG_DIR>/runtime.json`, the single-purpose
 * record of claw-server's currently-bound base URL. External processes
 * (the BrowserClaw Claude Desktop extension, primarily) read this file
 * to discover the running port without probing or scanning.
 *
 * Contract:
 *   - Written exactly once per successful bind, right after Bun.serve
 *     returns. If a subsequent bind on the same port happens (rebind,
 *     dev reload), the file is atomically overwritten with the new URL.
 *   - Atomic on POSIX: writes to `runtime.json.tmp` then renames. A
 *     concurrent reader either sees the previous content or the new
 *     content, never a torn write.
 *   - NOT deleted on shutdown. SIGKILL or a crash can't clean up
 *     anyway, and readers are expected to health-probe the URL before
 *     trusting it. Leaving a stale file behind is harmless.
 *   - Failures are logged, not thrown. The runtime file is a nice-to
 *     have; a claw-server that failed to write it still serves.
 */

import { mkdir, rename, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

import { getClawServerDir } from './browserclaw-dir'
import { logger } from './logger'

const RUNTIME_FILE = 'runtime.json'

export async function writeRuntimeFile(url: string): Promise<void> {
  const dir = getClawServerDir()
  const path = join(dir, RUNTIME_FILE)
  const tmp = `${path}.tmp`
  const payload = `${JSON.stringify({ url }, null, 2)}\n`
  try {
    await mkdir(dir, { recursive: true })
    await writeFile(tmp, payload, { encoding: 'utf8' })
    await rename(tmp, path)
  } catch (err) {
    logger.warn('runtime file write failed', {
      path,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}
