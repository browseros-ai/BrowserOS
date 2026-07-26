/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { readdir, readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'

/** Checks whether npm's npx cache already holds the configured ACP package range. */
export async function probeNpxPackageCache(
  packageName: string,
  options: { npxCacheDir?: string; versionRange?: string } = {},
): Promise<boolean> {
  const npxCacheDir = options.npxCacheDir ?? join(homedir(), '.npm', '_npx')
  let entries: Array<{ isDirectory(): boolean; name: string }>
  try {
    entries = await readdir(npxCacheDir, { withFileTypes: true })
  } catch {
    return false
  }

  const packageParts = packageName.split('/').filter(Boolean)
  for (const entry of entries) {
    if (!entry.isDirectory()) continue
    const candidate = join(
      npxCacheDir,
      entry.name,
      'node_modules',
      ...packageParts,
      'package.json',
    )
    try {
      const packageJson = await readFile(candidate, 'utf8')
      if (cachedPackageMatches(packageJson, options.versionRange)) return true
    } catch {
      // keep scanning
    }
  }
  return false
}

function cachedPackageMatches(
  packageJson: string,
  versionRange?: string,
): boolean {
  if (!versionRange) return true

  let version: unknown
  try {
    version = (JSON.parse(packageJson) as { version?: unknown }).version
  } catch {
    return false
  }
  return typeof version === 'string' && versionSatisfies(version, versionRange)
}

function versionSatisfies(version: string, range: string): boolean {
  const trimmedRange = range.trim()
  if (!trimmedRange) return true

  const candidate = parseSemver(version)
  if (!candidate) return false

  if (trimmedRange.startsWith('^')) {
    const base = parseSemver(trimmedRange.slice(1))
    if (!base) return false
    return (
      compareSemver(candidate, base) >= 0 &&
      compareSemver(candidate, caretUpperBound(base)) < 0
    )
  }

  const exact = parseSemver(trimmedRange)
  return !!exact && compareSemver(candidate, exact) === 0
}

function parseSemver(
  value: string,
): { major: number; minor: number; patch: number } | null {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(value.trim())
  if (!match) return null
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
  }
}

function compareSemver(
  left: { major: number; minor: number; patch: number },
  right: { major: number; minor: number; patch: number },
): number {
  return (
    left.major - right.major ||
    left.minor - right.minor ||
    left.patch - right.patch
  )
}

function caretUpperBound(version: {
  major: number
  minor: number
  patch: number
}): { major: number; minor: number; patch: number } {
  if (version.major > 0) {
    return { major: version.major + 1, minor: 0, patch: 0 }
  }
  if (version.minor > 0) {
    return { major: 0, minor: version.minor + 1, patch: 0 }
  }
  return { major: 0, minor: 0, patch: version.patch + 1 }
}
