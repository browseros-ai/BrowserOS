/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Writes Firefox logins into neo's password manager. CDP has no password
 * store domain that can add and update in bulk (BrowserOS's
 * `Autofill.addCredential` needs a live page of each site and refuses to
 * change a password), so the sync drives `chrome.passwordsPrivate`, the API
 * behind neo's own chrome://password-manager page, through a background tab:
 *
 *   getSavedPasswordList -> plan (add / update / unchanged / skip) -> addPassword | changeCredential
 *
 * Repeat syncs are decided by Firefox's `timePasswordChanged` remembered per
 * login GUID (sync-state.ts), so neither side's passwords are compared or
 * stored outside the two browsers.
 */

import type { FirefoxLogin } from '../firefox/logins'
import type { CdpConnection } from './cdp'

const PASSWORD_MANAGER_URL = 'chrome://password-manager/passwords'
const API_WAIT_MS = 10_000

/** A neo credential as getSavedPasswordList returns it (never a password). */
export interface SavedCredential {
  id: number
  username: string
  signonRealms: string[]
  /** The full PasswordUiEntry, echoed back to changeCredential. */
  entry: Record<string, unknown>
}

export type PasswordSkipReason =
  | 'http-auth'
  | 'unsupported-origin'
  | 'empty-password'
  | 'duplicate'
  | 'removed-in-neo'

export type PasswordAction =
  | { kind: 'add'; login: FirefoxLogin; realm: string }
  | { kind: 'update'; login: FirefoxLogin; saved: SavedCredential }
  | { kind: 'unchanged'; login: FirefoxLogin }
  | { kind: 'skip'; login: FirefoxLogin; reason: PasswordSkipReason }

/** What the last successful sync wrote, keyed by Firefox login GUID. */
export type PasswordSyncState = Record<string, { timePasswordChanged: number }>

/** Chromium's signon_realm for an HTML form login: the origin plus `/`. */
export function signonRealmFor(origin: string): string | undefined {
  try {
    const url = new URL(origin)
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return undefined
    return `${url.origin}/`
  } catch {
    return undefined
  }
}

const credentialKey = (realm: string, username: string) =>
  `${realm}\u0000${username}`

export function planPasswordSync(
  logins: readonly FirefoxLogin[],
  saved: readonly SavedCredential[],
  state: PasswordSyncState,
): PasswordAction[] {
  const neo = new Map<string, SavedCredential>()
  for (const credential of saved) {
    for (const realm of credential.signonRealms) {
      neo.set(credentialKey(realm, credential.username), credential)
    }
  }
  // Firefox may hold one origin+username twice (different form actions);
  // neo keys credentials without the form action, so the newest one wins.
  const newestFirst = [...logins].sort(
    (a, b) => b.timePasswordChanged - a.timePasswordChanged,
  )
  const seen = new Set<string>()
  const actions: PasswordAction[] = []
  for (const login of newestFirst) {
    const realm = signonRealmFor(login.origin)
    if (login.httpRealm !== null) {
      actions.push({ kind: 'skip', login, reason: 'http-auth' })
      continue
    }
    if (!realm) {
      actions.push({ kind: 'skip', login, reason: 'unsupported-origin' })
      continue
    }
    if (!login.password) {
      actions.push({ kind: 'skip', login, reason: 'empty-password' })
      continue
    }
    const key = credentialKey(realm, login.username)
    if (seen.has(key)) {
      actions.push({ kind: 'skip', login, reason: 'duplicate' })
      continue
    }
    seen.add(key)
    const previous = state[login.guid]
    const changedInFirefox =
      previous?.timePasswordChanged !== login.timePasswordChanged
    const existing = neo.get(key)
    if (existing) {
      actions.push(
        changedInFirefox
          ? { kind: 'update', login, saved: existing }
          : { kind: 'unchanged', login },
      )
    } else if (previous && !changedInFirefox) {
      // Synced before and deleted in neo since: respect the deletion until
      // the password changes in Firefox again.
      actions.push({ kind: 'skip', login, reason: 'removed-in-neo' })
    } else {
      actions.push({ kind: 'add', login, realm })
    }
  }
  return actions
}

/** A background chrome://password-manager tab whose page API does the writes. */
export class PasswordManagerPage {
  private constructor(
    private readonly cdp: CdpConnection,
    private readonly targetId: string,
    private readonly sessionId: string,
  ) {}

  static async open(cdp: CdpConnection): Promise<PasswordManagerPage> {
    const { targetId } = await cdp.send<{ targetId: string }>(
      'Target.createTarget',
      { url: PASSWORD_MANAGER_URL, background: true },
    )
    const { sessionId } = await cdp.send<{ sessionId: string }>(
      'Target.attachToTarget',
      { targetId, flatten: true },
    )
    const page = new PasswordManagerPage(cdp, targetId, sessionId)
    const deadline = Date.now() + API_WAIT_MS
    while (Date.now() < deadline) {
      const ready = await page
        .evaluate<boolean>(
          "typeof chrome !== 'undefined' && typeof chrome.passwordsPrivate?.addPassword === 'function'",
        )
        .catch(() => false)
      if (ready) return page
      await Bun.sleep(250)
    }
    await page.close()
    throw new Error(
      `${PASSWORD_MANAGER_URL} did not expose chrome.passwordsPrivate; this neo build cannot take passwords over CDP.`,
    )
  }

  private async evaluate<T>(expression: string): Promise<T> {
    const response = await this.cdp.send<{
      result: { value?: T }
      exceptionDetails?: { exception?: { description?: string }; text: string }
    }>(
      'Runtime.evaluate',
      { expression, awaitPromise: true, returnByValue: true },
      this.sessionId,
    )
    if (response.exceptionDetails) {
      const details = response.exceptionDetails
      throw new Error(details.exception?.description ?? details.text)
    }
    return response.result.value as T
  }

  list(): Promise<SavedCredential[]> {
    return this.evaluate<SavedCredential[]>(`
      chrome.passwordsPrivate.getSavedPasswordList().then((entries) =>
        entries
          .filter((e) => !e.isPasskey)
          .map((e) => ({
            id: e.id,
            username: e.username,
            signonRealms: e.affiliatedDomains.map((d) => d.signonRealm),
            entry: e,
          })))`)
  }

  async add(realm: string, login: FirefoxLogin): Promise<void> {
    const options = {
      url: realm,
      username: login.username,
      password: login.password,
      note: '',
      useAccountStore: false,
    }
    await this.evaluate(
      `chrome.passwordsPrivate.addPassword(${JSON.stringify(options)})`,
    )
  }

  async change(saved: SavedCredential, password: string): Promise<void> {
    const entry = { ...saved.entry, password }
    await this.evaluate(
      `chrome.passwordsPrivate.changeCredential(${JSON.stringify(entry)})`,
    )
  }

  async close(): Promise<void> {
    await this.cdp
      .send('Target.closeTarget', { targetId: this.targetId })
      .catch(() => undefined)
  }
}

export interface PasswordWriteReport {
  added: number
  updated: number
  unchanged: number
  skipped: Partial<Record<PasswordSkipReason, number>>
  /** Origins whose write neo refused, with the browser's message. */
  failed: { origin: string; error: string }[]
}

export async function applyPasswordPlan(
  page: PasswordManagerPage,
  actions: readonly PasswordAction[],
  state: PasswordSyncState,
): Promise<PasswordWriteReport> {
  const report: PasswordWriteReport = {
    added: 0,
    updated: 0,
    unchanged: 0,
    skipped: {},
    failed: [],
  }
  for (const action of actions) {
    const { login } = action
    try {
      if (action.kind === 'add') {
        await page.add(action.realm, login)
        report.added++
      } else if (action.kind === 'update') {
        await page.change(action.saved, login.password)
        report.updated++
      } else if (action.kind === 'unchanged') {
        report.unchanged++
        continue
      } else {
        report.skipped[action.reason] = (report.skipped[action.reason] ?? 0) + 1
        continue
      }
      // Only a write neo accepted moves the remembered change time forward.
      state[login.guid] = { timePasswordChanged: login.timePasswordChanged }
    } catch (error) {
      report.failed.push({
        origin: login.origin,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
  return report
}
