/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { statSync } from 'node:fs'
import { join } from 'node:path'

const BUNDLED_NATIVE_CLI_DIR_RELATIVE_PATH = join('bin', 'third_party')

export function withBundledNativeBinaryPath(input: {
  resourcesDir?: string | null
  env: Record<string, string>
  platform?: NodeJS.Platform
}): Record<string, string> {
  const resourcesDir = input.resourcesDir?.trim()
  if (!resourcesDir) return { ...input.env }

  const dir = join(resourcesDir, BUNDLED_NATIVE_CLI_DIR_RELATIVE_PATH)
  try {
    if (!statSync(dir).isDirectory()) return { ...input.env }
  } catch {
    return { ...input.env }
  }

  const platform = input.platform ?? process.platform
  const env = { ...input.env }
  const key = pathEnvKey(env, platform)
  const delimiter = platform === 'win32' ? ';' : ':'
  const existing = env[key] ?? ''
  const parts = existing
    .split(delimiter)
    .filter(Boolean)
    .filter((part) => part !== dir)
  env[key] = [dir, ...parts].join(delimiter)
  return env
}

function pathEnvKey(
  env: Record<string, string | undefined>,
  platform: NodeJS.Platform,
): string {
  if (platform !== 'win32') return 'PATH'
  return Object.keys(env).find((key) => key.toLowerCase() === 'path') ?? 'Path'
}
