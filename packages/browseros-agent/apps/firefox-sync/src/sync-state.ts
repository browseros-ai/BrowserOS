/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Remembers what earlier syncs wrote so a repeat sync only touches what
 * changed in Firefox and respects deletions made in neo. It holds login
 * GUIDs and timestamps only, never usernames, passwords or cookie values.
 * One entry per Firefox profile path, so syncing two profiles stays apart.
 */

import {
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  writeFileSync,
} from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import type { PasswordSyncState } from './neo/passwords'

export interface ProfileSyncState {
  passwords: PasswordSyncState
  lastSync?: string
}

interface StateFile {
  version: 1
  profiles: Record<string, ProfileSyncState>
}

export function defaultStatePath(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): string {
  const base =
    platform === 'win32'
      ? (env.LOCALAPPDATA ?? join(homedir(), 'AppData', 'Local'))
      : (env.XDG_STATE_HOME ?? join(homedir(), '.local', 'state'))
  return join(base, 'browseros-firefox-sync', 'state.json')
}

function readStateFile(path: string): StateFile {
  if (!existsSync(path)) return { version: 1, profiles: {} }
  const parsed = JSON.parse(readFileSync(path, 'utf8')) as StateFile
  return parsed.version === 1 ? parsed : { version: 1, profiles: {} }
}

export function loadProfileState(
  path: string,
  profilePath: string,
): ProfileSyncState {
  return readStateFile(path).profiles[profilePath] ?? { passwords: {} }
}

export function saveProfileState(
  path: string,
  profilePath: string,
  state: ProfileSyncState,
): void {
  const file = readStateFile(path)
  file.profiles[profilePath] = state
  mkdirSync(dirname(path), { recursive: true })
  // Write-then-rename: an interrupted sync never leaves a torn state file
  // that would make the next run re-add deleted passwords.
  const temp = `${path}.tmp`
  writeFileSync(temp, `${JSON.stringify(file, null, 2)}\n`)
  renameSync(temp, path)
}
