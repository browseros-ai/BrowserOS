/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  accessSync,
  chmodSync,
  constants,
  mkdirSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join } from 'node:path'

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

/**
 * Builds the environment used to launch Node-shebang ACP adapter
 * packages through BrowserOS's bundled Bun. Several adapter packages
 * publish `#!/usr/bin/env node` entrypoints; GUI-launched apps often
 * lack the user's Homebrew/nvm Node on PATH, so provide a private
 * `node` shim that execs bundled Bun.
 */
export function withBundledBunAcpAdapterEnv(input: {
  bunPath: string
  browserosDir?: string | null
  env?: Record<string, string | undefined>
  platform?: NodeJS.Platform
}): Record<string, string> {
  const platform = input.platform ?? process.platform
  const sourceEnv = input.env ?? process.env
  const env = input.env ? stringEnv(input.env) : {}
  const pathKey = pathEnvKey(sourceEnv, platform)
  const delimiter = platform === 'win32' ? ';' : ':'
  const pathEntries = [
    ensureBundledNodeShim({
      bunPath: input.bunPath,
      browserosDir: input.browserosDir,
      platform,
    }),
    dirname(input.bunPath),
    ...(sourceEnv[pathKey] ?? '').split(delimiter),
  ].filter((entry): entry is string => Boolean(entry))

  env[pathKey] = dedupe(pathEntries).join(delimiter)
  const browserosDir = input.browserosDir?.trim()
  if (browserosDir) {
    env.BUN_INSTALL_CACHE_DIR = join(browserosDir, 'cache', 'bun-install')
  }
  return env
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

function ensureBundledNodeShim(input: {
  bunPath: string
  browserosDir?: string | null
  platform: NodeJS.Platform
}): string | null {
  if (input.platform === 'win32') return null
  const browserosDir = input.browserosDir?.trim()
  if (!browserosDir) return null

  const shimDir = join(browserosDir, 'cache', 'acp-node-shim')
  const shimPath = join(shimDir, 'node')
  try {
    mkdirSync(shimDir, { recursive: true })
    writeFileSync(
      shimPath,
      `#!/bin/sh\nexec ${shellQuote(input.bunPath)} "$@"\n`,
      'utf8',
    )
    chmodSync(shimPath, 0o755)
    return shimDir
  } catch {
    return null
  }
}

function stringEnv(
  env: Record<string, string | undefined>,
): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [key, value] of Object.entries(env)) {
    if (value !== undefined) out[key] = value
  }
  return out
}

function pathEnvKey(
  env: Record<string, string | undefined>,
  platform: NodeJS.Platform,
): string {
  if (platform !== 'win32') return 'PATH'
  return Object.keys(env).find((key) => key.toLowerCase() === 'path') ?? 'Path'
}

function dedupe(values: string[]): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (const value of values) {
    if (seen.has(value)) continue
    seen.add(value)
    out.push(value)
  }
  return out
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`
}
