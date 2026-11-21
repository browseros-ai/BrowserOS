/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Finds Firefox profiles from profiles.ini. Since Firefox 67 each install
 * (release, Developer Edition, ...) records its own default in an
 * `[Install<hash>]` section; the legacy `Default=1` flag on a `[ProfileN]`
 * section is often a stale profile. Install defaults therefore win, and among
 * several installs the most recently used profile is the one to sync.
 */

import { existsSync, readFileSync, statSync } from 'node:fs'
import { homedir } from 'node:os'
import { basename, isAbsolute, join, normalize } from 'node:path'

export interface FirefoxProfile {
  name: string
  path: string
  /** Default of at least one Firefox install (`[Install...]` section). */
  installDefault: boolean
  /** Legacy `Default=1` flag. */
  legacyDefault: boolean
}

export function firefoxRootDir(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): string {
  if (platform === 'win32') {
    return join(
      env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'),
      'Mozilla',
      'Firefox',
    )
  }
  if (platform === 'darwin') {
    return join(homedir(), 'Library', 'Application Support', 'Firefox')
  }
  return join(homedir(), '.mozilla', 'firefox')
}

function parseIni(text: string): Map<string, Map<string, string>> {
  const sections = new Map<string, Map<string, string>>()
  let current: Map<string, string> | undefined
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || line.startsWith(';') || line.startsWith('#')) continue
    const header = /^\[(.+)\]$/.exec(line)
    if (header) {
      current = new Map()
      sections.set(header[1], current)
      continue
    }
    const eq = line.indexOf('=')
    if (current && eq > 0) {
      current.set(line.slice(0, eq).trim(), line.slice(eq + 1).trim())
    }
  }
  return sections
}

function resolveProfilePath(root: string, path: string, relative: boolean) {
  return normalize(relative ? join(root, path) : path)
}

export function parseProfilesIni(text: string, root: string): FirefoxProfile[] {
  const sections = parseIni(text)
  const installDefaults = new Set<string>()
  for (const [name, values] of sections) {
    const target = values.get('Default')
    if (name.startsWith('Install') && target) {
      installDefaults.add(resolveProfilePath(root, target, !isAbsolute(target)))
    }
  }
  const profiles: FirefoxProfile[] = []
  for (const [name, values] of sections) {
    const path = values.get('Path')
    if (!name.startsWith('Profile') || !path) continue
    const resolved = resolveProfilePath(
      root,
      path,
      values.get('IsRelative') !== '0',
    )
    profiles.push({
      name: values.get('Name') ?? basename(resolved),
      path: resolved,
      installDefault: installDefaults.has(resolved),
      legacyDefault: values.get('Default') === '1',
    })
  }
  return profiles
}

export function listFirefoxProfiles(root = firefoxRootDir()): FirefoxProfile[] {
  const ini = join(root, 'profiles.ini')
  if (!existsSync(ini)) return []
  return parseProfilesIni(readFileSync(ini, 'utf8'), root)
}

/** Last time Firefox wrote session state into the profile. */
function lastUsed(profile: FirefoxProfile): number {
  for (const file of ['cookies.sqlite-wal', 'cookies.sqlite', 'prefs.js']) {
    const path = join(profile.path, file)
    if (existsSync(path)) return statSync(path).mtimeMs
  }
  return 0
}

export function pickDefaultProfile(
  profiles: FirefoxProfile[],
  usedAt: (profile: FirefoxProfile) => number = lastUsed,
): FirefoxProfile | undefined {
  const installDefaults = profiles.filter((p) => p.installDefault)
  if (installDefaults.length > 0) {
    return [...installDefaults].sort((a, b) => usedAt(b) - usedAt(a))[0]
  }
  return profiles.find((p) => p.legacyDefault) ?? profiles[0]
}

/** Accepts a profile name, its directory name, or a full path. */
export function resolveProfile(
  query: string | undefined,
  profiles: FirefoxProfile[],
): FirefoxProfile | undefined {
  if (!query) return pickDefaultProfile(profiles)
  if (isAbsolute(query) && existsSync(query)) {
    const path = normalize(query)
    return (
      profiles.find((p) => p.path === path) ?? {
        name: basename(path),
        path,
        installDefault: false,
        legacyDefault: false,
      }
    )
  }
  return profiles.find((p) => p.name === query || basename(p.path) === query)
}
