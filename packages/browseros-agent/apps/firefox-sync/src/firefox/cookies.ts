/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Reads Firefox cookies into a browser-neutral shape. Persistent cookies come
 * from cookies.sqlite; cookies without an expiry ("session" cookies, which
 * many sites use for the signed-in state) only exist in the session store
 * (see sessionstore.ts). Cookies outside the default jar — containers,
 * private windows and Total Cookie Protection partitions — are reported, not
 * merged: Chromium has one jar per profile, and mixing jars would mix
 * accounts.
 */

import { withSqliteSnapshot } from './snapshot'

export type SameSite = 'None' | 'Lax' | 'Strict'

export type SkipReason =
  | 'container'
  | 'partitioned'
  | 'private'
  | 'origin-attributes'
  | 'expired'

export interface FirefoxCookie {
  name: string
  value: string
  /** Leading dot = domain cookie; no dot = host-only cookie. */
  host: string
  path: string
  /** Milliseconds since the epoch; null for a session cookie. */
  expiresMs: number | null
  secure: boolean
  httpOnly: boolean
  sameSite?: SameSite
  /** Firefox scheme bits: 1 = http, 2 = https, 4 = file. */
  schemeMap: number
}

export interface CookieReadResult {
  cookies: FirefoxCookie[]
  skipped: Partial<Record<SkipReason, number>>
}

// nsICookie::SAMESITE_*; any other value (unset) keeps Chromium's default.
const SAME_SITE: Record<number, SameSite> = { 0: 'None', 1: 'Lax', 2: 'Strict' }

export function sameSiteFromFirefox(value: number | undefined) {
  return value === undefined ? undefined : SAME_SITE[value]
}

/**
 * cookies.sqlite stored `expiry` in seconds until schema 16 and in
 * milliseconds after it. Seconds stay below 1e11 until the year 5138.
 */
export function expiryToMs(expiry: number): number {
  return expiry < 1e11 ? expiry * 1000 : expiry
}

/**
 * Classifies a serialized originAttributes suffix such as
 * `^userContextId=2&partitionKey=%28https%2Cexample.com%29`.
 */
export function originAttributesSkipReason(
  suffix: string,
): SkipReason | undefined {
  if (!suffix) return undefined
  const params = new URLSearchParams(suffix.replace(/^\^/, ''))
  if (params.get('partitionKey')) return 'partitioned'
  if (params.get('userContextId') && params.get('userContextId') !== '0') {
    return 'container'
  }
  if (
    params.get('privateBrowsingId') &&
    params.get('privateBrowsingId') !== '0'
  ) {
    return 'private'
  }
  return 'origin-attributes'
}

interface CookieRow {
  originAttributes: string
  name: string
  value: string
  host: string
  path: string
  expiry: number
  isSecure: number
  isHttpOnly: number
  sameSite: number
  schemeMap: number
}

export function readCookieDatabase(
  profileDir: string,
  now = Date.now(),
): CookieReadResult {
  const rows = withSqliteSnapshot(
    profileDir,
    'cookies.sqlite',
    (db) =>
      db
        .query(
          'SELECT originAttributes, name, value, host, path, expiry, isSecure, isHttpOnly, sameSite, schemeMap FROM moz_cookies',
        )
        .all() as CookieRow[],
  )
  const result: CookieReadResult = { cookies: [], skipped: {} }
  for (const row of rows) {
    const reason =
      originAttributesSkipReason(row.originAttributes) ??
      (expiryToMs(row.expiry) <= now ? 'expired' : undefined)
    if (reason) {
      result.skipped[reason] = (result.skipped[reason] ?? 0) + 1
      continue
    }
    result.cookies.push({
      name: row.name,
      value: row.value,
      host: row.host,
      path: row.path || '/',
      expiresMs: expiryToMs(row.expiry),
      secure: row.isSecure === 1,
      httpOnly: row.isHttpOnly === 1,
      sameSite: sameSiteFromFirefox(row.sameSite),
      schemeMap: row.schemeMap,
    })
  }
  return result
}
