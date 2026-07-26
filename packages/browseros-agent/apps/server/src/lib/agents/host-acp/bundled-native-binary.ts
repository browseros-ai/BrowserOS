/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { accessSync, constants, statSync } from 'node:fs'
import { join } from 'node:path'
import {
  buildResolvedBinaryEnv,
  type ResolvedHostBinary,
} from './binary-resolver'
import type { HostAcpAdapter } from './config'

const BUNDLED_NATIVE_CLI_DIR_RELATIVE_PATH = join('bin', 'third_party')

/** Resolves BrowserOS-packaged Claude/Codex binaries before falling back to host PATH. */
export function resolveBundledNativeBinary(input: {
  adapter: HostAcpAdapter
  resourcesDir?: string | null
  env?: NodeJS.ProcessEnv
  platform?: NodeJS.Platform
}): ResolvedHostBinary | null {
  const resourcesDir = input.resourcesDir?.trim()
  if (!resourcesDir) return null

  const platform = input.platform ?? process.platform
  const candidate = join(
    resourcesDir,
    BUNDLED_NATIVE_CLI_DIR_RELATIVE_PATH,
    bundledBinaryName(input.adapter, platform),
  )
  if (!isUsableBundledBinary(candidate, platform)) return null

  return {
    path: candidate,
    env: buildResolvedBinaryEnv({
      binaryPath: candidate,
      env: input.env,
      platform,
    }),
  }
}

/** Prepends BrowserOS's packaged CLI directory so ACP packages spawn bundled CLIs first. */
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

function bundledBinaryName(
  adapter: HostAcpAdapter,
  platform: NodeJS.Platform,
): string {
  return platform === 'win32' ? `${adapter}.exe` : adapter
}

function isUsableBundledBinary(
  path: string,
  platform: NodeJS.Platform,
): boolean {
  try {
    if (!statSync(path).isFile()) return false
    if (platform !== 'win32') accessSync(path, constants.X_OK)
    return true
  } catch {
    return false
  }
}

function pathEnvKey(
  env: Record<string, string | undefined>,
  platform: NodeJS.Platform,
): string {
  if (platform !== 'win32') return 'PATH'
  return Object.keys(env).find((key) => key.toLowerCase() === 'path') ?? 'Path'
}
