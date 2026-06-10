/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { accessSync, constants, statSync } from 'node:fs'
import { join } from 'node:path'

const BUNDLED_BUN_RELATIVE_PATH = join('bin', 'third_party', 'bun')
const WINDOWS_BUNDLED_BUN_RELATIVE_PATH = join('bin', 'third_party', 'bun.exe')

/** Resolves the packaged Bun executable used to run ACP adapter packages. */
export function resolveBundledBun(input: {
  resourcesDir?: string | null
  platform?: NodeJS.Platform
}): string | null {
  const platform = input.platform ?? process.platform
  const relativePath = bundledBunRelativePath(platform)
  if (!relativePath) return null
  const resourcesDir = input.resourcesDir?.trim()
  if (!resourcesDir) return null

  const candidate = join(resourcesDir, relativePath)
  try {
    if (!statSync(candidate).isFile()) return null
    if (platform !== 'win32') {
      accessSync(candidate, constants.X_OK)
    }
    return candidate
  } catch {
    return null
  }
}

function bundledBunRelativePath(platform: NodeJS.Platform): string | null {
  if (platform === 'darwin' || platform === 'linux') {
    return BUNDLED_BUN_RELATIVE_PATH
  }
  if (platform === 'win32') {
    return WINDOWS_BUNDLED_BUN_RELATIVE_PATH
  }
  return null
}
