/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * What the onboarding chat offers to bring over from another browser, in the
 * person's words. Everything here derives from the Chromium import source the
 * person picked: one chip per `supportedItems`, so a profile (or a platform)
 * that cannot bring logins simply never shows those chips.
 */

import type {
  BrowserOSImportItem,
  BrowserOSImportSource,
  BrowserOSStartImportRequest,
} from './browseros-onboarding-api'

/** Display order: logins first, then the everyday data, then the opt-ins. */
const CHIP_ORDER: readonly BrowserOSImportItem[] = [
  'cookies',
  'passwords',
  'bookmarks',
  'history',
  'autofill',
  'extensions',
  'searchEngines',
]

const IMPORT_ITEM_LABELS: Record<BrowserOSImportItem, string> = {
  cookies: 'Signed-in sites',
  passwords: 'Passwords',
  bookmarks: 'Bookmarks',
  history: 'History',
  autofill: 'Autofill',
  extensions: 'Extensions',
  searchEngines: 'Search engines',
}

/** Chromium and these resources ship separately, so a newer native item still
 * needs a readable label rather than its camelCase id. */
function humanizeImportItem(item: string): string {
  const label = item
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[-_]+/g, ' ')
    .trim()
  if (!label) return 'Other data'
  return label.charAt(0).toUpperCase() + label.slice(1).toLowerCase()
}

export function importItemLabel(item: string): string {
  return (
    IMPORT_ITEM_LABELS[item as BrowserOSImportItem] ?? humanizeImportItem(item)
  )
}

/** Opt-in items stay off until tapped; they change how the browser behaves. */
export const OPTIONAL_IMPORT_ITEMS: readonly BrowserOSImportItem[] = [
  'searchEngines',
  'extensions',
]

const LOGIN_ITEMS: readonly BrowserOSImportItem[] = ['cookies', 'passwords']

function isOptionalImportItem(item: BrowserOSImportItem): boolean {
  return OPTIONAL_IMPORT_ITEMS.includes(item)
}

export interface ImportChip {
  item: BrowserOSImportItem
  label: string
  /** Optional chips sit in their own row so toggling never moves a chip. */
  optional: boolean
}

export function importChipsFor(source: BrowserOSImportSource): ImportChip[] {
  const rank = (item: BrowserOSImportItem) => {
    const index = CHIP_ORDER.indexOf(item)
    return index === -1 ? CHIP_ORDER.length : index
  }
  return [...new Set(source.supportedItems)]
    .sort((a, b) => rank(a) - rank(b))
    .map((item) => ({
      item,
      label: importItemLabel(item),
      optional: isOptionalImportItem(item),
    }))
}

/**
 * Native reports `recommendedItems` identical to `supportedItems`, so the
 * default is every supported item except the opt-ins (#2120). Following
 * `recommendedItems` alone would switch Extensions and Search engines on.
 */
export function defaultImportItemsForSource(
  source: BrowserOSImportSource,
): BrowserOSImportItem[] {
  return source.supportedItems.filter((item) => !isOptionalImportItem(item))
}

export function sanitizeImportSelection(
  source: BrowserOSImportSource,
  items: readonly BrowserOSImportItem[],
): BrowserOSImportItem[] {
  const selectedItems = new Set(items)
  return [...new Set(source.supportedItems)].filter((item) =>
    selectedItems.has(item),
  )
}

export function startImportRequestFor(
  source: BrowserOSImportSource,
  items?: readonly BrowserOSImportItem[],
): BrowserOSStartImportRequest | null {
  const importItems =
    items === undefined
      ? defaultImportItemsForSource(source)
      : sanitizeImportSelection(source, items)
  if (importItems.length === 0) return null
  return { sourceId: source.id, items: importItems }
}

export function includesLogins(items: readonly BrowserOSImportItem[]): boolean {
  return items.some((item) => LOGIN_ITEMS.includes(item))
}

/** "Google Chrome" reads as "Chrome" in the chat; other browsers keep their name. */
export function browserShortName(source: BrowserOSImportSource): string {
  return source.browserName.replace(/^Google /, '') || 'your browser'
}

/** The line neo ends the import on. It names only what native reported done. */
export function importDoneLine(
  completedItems: readonly BrowserOSImportItem[],
): string {
  if (includesLogins(completedItems)) return 'Done. Your logins are here.'
  const done = new Set(completedItems)
  // Round 4's line for every import without logins, whenever it is true.
  if (done.has('bookmarks') && done.has('history')) {
    return 'Done. Your bookmarks and history are here.'
  }
  const labels = [...done].map((item) => importItemLabel(item).toLowerCase())
  if (labels.length === 0) return 'Done.'
  if (labels.length === 1) {
    // "history" and "autofill" are mass nouns; the other labels are plural.
    const verb = labels[0].endsWith('s') ? 'are' : 'is'
    return `Done. Your ${labels[0]} ${verb} here.`
  }
  const list = `${labels.slice(0, -1).join(', ')} and ${labels.at(-1)}`
  return `Done. Your ${list} are here.`
}

export type ClientPlatform = 'mac' | 'windows' | 'other'

/** The native API omits platform; use browser platform hints, never source IDs. */
export function detectPlatform(): ClientPlatform {
  if (typeof navigator === 'undefined') return 'other'
  const client = navigator as Navigator & {
    userAgentData?: { platform?: string }
  }
  const platform = client.userAgentData?.platform || client.platform
  if (/^(macOS|MacIntel|MacPPC|Mac68K)$/i.test(platform)) return 'mac'
  if (/^Win/i.test(platform)) return 'windows'
  return 'other'
}

/**
 * What the chat may offer from a source on this platform. On Windows the
 * patch lists cookies and passwords whenever Chrome's files exist, but the
 * importer silently skips Chrome 127+ App-Bound (v20) values, so logins are
 * never offered or sent there (owner, round 4). Dropping them natively is a
 * follow-up Chromium change; until then this is the only guard.
 */
export function offeredSource(
  source: BrowserOSImportSource,
  platform: ClientPlatform,
): BrowserOSImportSource {
  if (platform !== 'windows') return source
  const keep = (item: BrowserOSImportItem) => !LOGIN_ITEMS.includes(item)
  return {
    ...source,
    supportedItems: source.supportedItems.filter(keep),
    recommendedItems: source.recommendedItems.filter(keep),
  }
}

/** macOS asks for the Keychain only when Chrome's encrypted data comes over. */
export function needsMacKeychainPermission(
  isMac: boolean,
  source: BrowserOSImportSource | undefined,
  selectedItems: readonly BrowserOSImportItem[],
): boolean {
  if (!isMac || !source || !/^(Google )?Chrome$/i.test(source.browserName))
    return false
  return includesLogins(sanitizeImportSelection(source, selectedItems))
}
