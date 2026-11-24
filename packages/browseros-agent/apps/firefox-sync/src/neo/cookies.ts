/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Writes Firefox cookies into neo's default profile with
 * `Storage.setCookies` on the browser target, which goes through the same
 * CookieManager the native importer uses (ProfileWriter::AddCookie). Writing
 * a cookie with an existing (name, domain, path) replaces it, so a repeat
 * sync updates sessions in place. After writing, the jar is read back: the
 * report counts cookies neo actually holds, not calls that returned.
 */

import type { FirefoxCookie } from '../firefox/cookies'
import type { CdpConnection } from './cdp'

const BATCH_SIZE = 100

/** CDP Network.CookieParam, restricted to the fields the sync sets. */
export interface CookieParam {
  name: string
  value: string
  url?: string
  domain?: string
  path: string
  secure: boolean
  httpOnly: boolean
  sameSite?: 'None' | 'Lax' | 'Strict'
  /** Seconds since the epoch; omitted for session cookies. */
  expires?: number
}

interface CdpCookie {
  name: string
  value: string
  domain: string
  path: string
}

export function cookieKey(c: { name: string; path: string }, domain: string) {
  return `${c.name}\u0000${domain}\u0000${c.path}`
}

/** Key in the domain form CDP reports: `.example.com` or `example.com`. */
export function paramKey(p: CookieParam): string {
  const domain = p.domain ?? new URL(p.url ?? '').hostname
  return cookieKey(p, domain)
}

export function toCookieParam(c: FirefoxCookie): CookieParam {
  const param: CookieParam = {
    name: c.name,
    value: c.value,
    path: c.path,
    secure: c.secure,
    httpOnly: c.httpOnly,
  }
  if (c.host.startsWith('.')) {
    param.domain = c.host
  } else {
    // A Domain attribute would widen a host-only cookie to subdomains;
    // a URL without a domain keeps it host-only, as in Firefox.
    const https = c.secure || (c.schemeMap & 2) !== 0 || c.schemeMap === 0
    param.url = `${https ? 'https' : 'http'}://${c.host}${c.path}`
  }
  // Chromium rejects SameSite=None without Secure; leaving it out keeps the
  // browser's default (Lax) instead of losing the cookie.
  if (c.sameSite && (c.sameSite !== 'None' || c.secure)) {
    param.sameSite = c.sameSite
  }
  if (c.expiresMs !== null) param.expires = c.expiresMs / 1000
  return param
}

export function hostMatches(host: string, domains: readonly string[]): boolean {
  if (domains.length === 0) return true
  const bare = host.replace(/^\./, '').toLowerCase()
  return domains.some((d) => {
    const domain = d.replace(/^\./, '').toLowerCase()
    return bare === domain || bare.endsWith(`.${domain}`)
  })
}

export interface CookieWriteReport {
  attempted: number
  confirmed: number
  /** Hosts whose cookies neo refused; values are never reported. */
  rejectedHosts: string[]
  keptExisting: number
}

async function readJar(cdp: CdpConnection): Promise<Map<string, string>> {
  const { cookies } = await cdp.send<{ cookies: CdpCookie[] }>(
    'Storage.getCookies',
  )
  return new Map(cookies.map((c) => [cookieKey(c, c.domain), c.value]))
}

async function setBatch(
  cdp: CdpConnection,
  batch: CookieParam[],
  rejectedHosts: Set<string>,
) {
  try {
    await cdp.send('Storage.setCookies', { cookies: batch })
  } catch {
    // One invalid cookie fails the whole call; retry one by one so a single
    // bad entry does not drop its neighbours.
    for (const cookie of batch) {
      try {
        await cdp.send('Storage.setCookies', { cookies: [cookie] })
      } catch {
        rejectedHosts.add(cookie.domain ?? new URL(cookie.url ?? '').hostname)
      }
    }
  }
}

export async function writeCookies(
  cdp: CdpConnection,
  params: CookieParam[],
  options: { missingOnly: boolean },
): Promise<CookieWriteReport> {
  let toWrite = params
  let keptExisting = 0
  if (options.missingOnly) {
    const jar = await readJar(cdp)
    toWrite = params.filter((p) => !jar.has(paramKey(p)))
    keptExisting = params.length - toWrite.length
  }
  const rejectedHosts = new Set<string>()
  for (let i = 0; i < toWrite.length; i += BATCH_SIZE) {
    await setBatch(cdp, toWrite.slice(i, i + BATCH_SIZE), rejectedHosts)
  }
  const jar = await readJar(cdp)
  const confirmed = toWrite.filter((p) => jar.get(paramKey(p)) === p.value)
  return {
    attempted: toWrite.length,
    confirmed: confirmed.length,
    rejectedHosts: [...rejectedHosts].sort(),
    keptExisting,
  }
}
