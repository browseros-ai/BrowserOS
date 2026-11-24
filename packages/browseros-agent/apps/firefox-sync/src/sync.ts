/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One sync run, Firefox -> neo:
 *
 *   Firefox profile (read-only snapshots)        running neo (CDP)
 *   cookies.sqlite + session store  --cookies--> Storage.setCookies, read back
 *   logins.json + key4.db           --logins---> chrome.passwordsPrivate (sync-state.ts decides)
 *
 * Firefox is never written to. Every run is safe to repeat: cookies replace
 * their namesakes, passwords are added or updated only when Firefox changed
 * them since the last run. Nothing is deleted on either side.
 */

import {
  type FirefoxCookie,
  readCookieDatabase,
  type SkipReason,
} from './firefox/cookies'
import { readFirefoxLogins } from './firefox/logins'
import type { FirefoxProfile } from './firefox/profiles'
import { readSessionCookies } from './firefox/sessionstore'
import { CdpConnection } from './neo/cdp'
import {
  cookieKey,
  hostMatches,
  paramKey,
  toCookieParam,
  writeCookies,
} from './neo/cookies'
import { findBrowser } from './neo/discover'
import {
  applyPasswordPlan,
  PasswordManagerPage,
  planPasswordSync,
} from './neo/passwords'
import { loadProfileState, saveProfileState } from './sync-state'

export interface SyncOptions {
  profile: FirefoxProfile
  cookies: boolean
  sessionCookies: boolean
  passwords: boolean
  /** Only sites under these domains; empty means every site. */
  domains: string[]
  /** Keep cookies neo already has instead of replacing them. */
  missingOnly: boolean
  dryRun: boolean
  primaryPassword?: string
  cdpPort?: number
  statePath: string
  log: (line: string) => void
}

function formatCounts(counts: Partial<Record<string, number>>): string {
  const parts = Object.entries(counts)
    .filter(([, n]) => n)
    .map(([reason, n]) => `${reason} ${n}`)
  return parts.length ? parts.join(', ') : 'none'
}

function mergeSkips(
  ...all: Partial<Record<SkipReason, number>>[]
): Partial<Record<SkipReason, number>> {
  const merged: Partial<Record<SkipReason, number>> = {}
  for (const skips of all) {
    for (const [reason, n] of Object.entries(skips) as [SkipReason, number][]) {
      merged[reason] = (merged[reason] ?? 0) + n
    }
  }
  return merged
}

function collectCookies(options: SyncOptions) {
  const persistent = readCookieDatabase(options.profile.path)
  const session = options.sessionCookies
    ? readSessionCookies(options.profile.path)
    : { cookies: [], skipped: {} }
  // A cookie present in both sources keeps the persistent copy.
  const byKey = new Map<string, FirefoxCookie>()
  for (const c of [...persistent.cookies, ...session.cookies]) {
    const key = cookieKey(c, c.host)
    if (!byKey.has(key)) byKey.set(key, c)
  }
  const cookies = [...byKey.values()].filter((c) =>
    hostMatches(c.host, options.domains),
  )
  return {
    params: cookies.map(toCookieParam),
    sessionCount: cookies.filter((c) => c.expiresMs === null).length,
    skipped: mergeSkips(persistent.skipped, session.skipped),
  }
}

async function syncCookies(
  cdp: CdpConnection | undefined,
  options: SyncOptions,
) {
  const { params, sessionCount, skipped } = collectCookies(options)
  options.log(
    `Firefox cookies: ${params.length} to sync (${sessionCount} session-only); skipped: ${formatCounts(skipped)}`,
  )
  if (!cdp) return true
  if (options.dryRun) {
    const { cookies } = await cdp.send<{
      cookies: { name: string; domain: string; path: string }[]
    }>('Storage.getCookies')
    const jar = new Set(cookies.map((c) => cookieKey(c, c.domain)))
    const existing = params.filter((p) => jar.has(paramKey(p))).length
    options.log(
      `neo cookies (dry run): ${params.length - existing} new, ${existing} ${options.missingOnly ? 'kept' : 'replaced'}`,
    )
    return true
  }
  const report = await writeCookies(cdp, params, options)
  options.log(
    `neo cookies: ${report.confirmed}/${report.attempted} confirmed in the jar` +
      (options.missingOnly ? `, ${report.keptExisting} existing kept` : ''),
  )
  if (report.rejectedHosts.length) {
    options.log(`  refused by neo for: ${report.rejectedHosts.join(', ')}`)
  }
  if (sessionCount) {
    options.log(
      '  session-only cookies last until neo closes, unless neo restores the previous session on startup',
    )
  }
  return report.confirmed === report.attempted
}

async function syncPasswords(
  cdp: CdpConnection | undefined,
  options: SyncOptions,
) {
  const { logins: all, undecryptable } = readFirefoxLogins(
    options.profile.path,
    options.primaryPassword,
  )
  const logins = all.filter((l) => {
    try {
      return hostMatches(new URL(l.origin).hostname, options.domains)
    } catch {
      return options.domains.length === 0
    }
  })
  options.log(
    `Firefox logins: ${logins.length} read` +
      (undecryptable ? `, ${undecryptable} could not be decrypted` : ''),
  )
  if (!cdp) return undecryptable === 0
  const state = loadProfileState(options.statePath, options.profile.path)
  const page = await PasswordManagerPage.open(cdp)
  try {
    const actions = planPasswordSync(logins, await page.list(), state.passwords)
    if (options.dryRun) {
      const counts: Record<string, number> = {}
      for (const a of actions) {
        const key = a.kind === 'skip' ? `skip:${a.reason}` : a.kind
        counts[key] = (counts[key] ?? 0) + 1
      }
      options.log(`neo passwords (dry run): ${formatCounts(counts)}`)
      return true
    }
    const report = await applyPasswordPlan(page, actions, state.passwords)
    state.lastSync = new Date().toISOString()
    saveProfileState(options.statePath, options.profile.path, state)
    options.log(
      `neo passwords: ${report.added} added, ${report.updated} updated, ${report.unchanged} unchanged; skipped: ${formatCounts(report.skipped)}`,
    )
    for (const failure of report.failed) {
      options.log(`  failed ${failure.origin}: ${failure.error}`)
    }
    return report.failed.length === 0 && undecryptable === 0
  } finally {
    await page.close()
  }
}

/** Returns true when everything selected reached neo. */
export async function runSync(options: SyncOptions): Promise<boolean> {
  options.log(
    `Firefox profile: ${options.profile.name} (${options.profile.path})`,
  )
  const endpoint = await findBrowser(options.cdpPort)
  if (!endpoint && !options.dryRun) {
    throw new Error(
      'No running BrowserOS neo found over CDP. Start neo, or pass --cdp-port.',
    )
  }
  if (endpoint) {
    options.log(`neo: ${endpoint.product} on CDP port ${endpoint.port}`)
  } else {
    options.log('neo is not running: showing the Firefox side only')
  }
  const cdp = endpoint ? await CdpConnection.open(endpoint.wsUrl) : undefined
  try {
    let ok = true
    if (options.cookies) ok = (await syncCookies(cdp, options)) && ok
    if (options.passwords) ok = (await syncPasswords(cdp, options)) && ok
    return ok
  } finally {
    cdp?.close()
  }
}
