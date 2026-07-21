/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { env } from '../../src/env'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../src/lib/mcp-manager'
import { createStubMcpManager } from './stub-mcp-manager'

/** Runs a test against an isolated BrowserClaw state root with MCP side effects stubbed. */
export async function withTempBrowserClawDir<T>(
  body: (dir: string) => Promise<T>,
): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), 'browserclaw-server-'))
  const prior = env.browserClawDirOverride
  env.browserClawDirOverride = dir
  setMcpManagerForTesting(createStubMcpManager())
  try {
    return await body(dir)
  } finally {
    env.browserClawDirOverride = prior
    resetMcpManagerForTesting()
    await rm(dir, { recursive: true, force: true })
  }
}
