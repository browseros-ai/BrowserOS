/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Reads Firefox saved logins into plain records. Firefox desktop keeps them in
 * logins.json; a logins.db next to it belongs to the application-services
 * store that desktop does not use, so it is ignored. Decrypted values only
 * live in memory and are never logged.
 */

import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { decryptLoginField, readLoginKeys } from './nss'
import { withSqliteSnapshot } from './snapshot'

export interface FirefoxLogin {
  guid: string
  /** Page origin, such as `https://example.com`. */
  origin: string
  /** Set for HTTP authentication logins instead of form logins. */
  httpRealm: string | null
  username: string
  password: string
  /** Milliseconds since the epoch; drives repeat-sync decisions. */
  timePasswordChanged: number
}

interface LoginsJsonEntry {
  guid: string
  hostname: string
  httpRealm: string | null
  encryptedUsername: string
  encryptedPassword: string
  timePasswordChanged?: number
  timeCreated?: number
}

export interface ReadLoginsResult {
  logins: FirefoxLogin[]
  /** Entries whose ciphertext did not decrypt with the profile key. */
  undecryptable: number
}

export function readFirefoxLogins(
  profileDir: string,
  primaryPassword?: string,
): ReadLoginsResult {
  const file = join(profileDir, 'logins.json')
  if (!existsSync(file)) return { logins: [], undecryptable: 0 }
  const entries = (
    JSON.parse(readFileSync(file, 'utf8')) as { logins?: LoginsJsonEntry[] }
  ).logins
  if (!entries?.length) return { logins: [], undecryptable: 0 }

  const keys = withSqliteSnapshot(profileDir, 'key4.db', (db) =>
    readLoginKeys(db, primaryPassword),
  )
  const logins: FirefoxLogin[] = []
  let undecryptable = 0
  for (const entry of entries) {
    try {
      logins.push({
        guid: entry.guid,
        origin: entry.hostname,
        httpRealm: entry.httpRealm ?? null,
        username: decryptLoginField(entry.encryptedUsername, keys),
        password: decryptLoginField(entry.encryptedPassword, keys),
        timePasswordChanged:
          entry.timePasswordChanged ?? entry.timeCreated ?? 0,
      })
    } catch {
      undecryptable++
    }
  }
  for (const key of keys) key.fill(0)
  return { logins, undecryptable }
}
