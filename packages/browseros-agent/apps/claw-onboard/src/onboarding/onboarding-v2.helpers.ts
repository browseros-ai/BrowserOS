/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  BrowserOSImportItem,
  BrowserOSImportProgress,
  BrowserOSImportSource,
  BrowserOSStartImportRequest,
} from './browseros-onboarding-api'

export const MOCK_BROWSEROS_IMPORT_SOURCES: readonly BrowserOSImportSource[] = [
  {
    id: 'chrome-work',
    displayName: 'Google Chrome - Work',
    browserName: 'Google Chrome',
    profileName: 'Work',
    supportedItems: [
      'history',
      'bookmarks',
      'cookies',
      'passwords',
      'searchEngines',
      'autofill',
      'extensions',
    ],
    recommendedItems: [
      'history',
      'bookmarks',
      'cookies',
      'passwords',
      'searchEngines',
      'autofill',
      'extensions',
    ],
  },
  {
    id: 'chrome-personal',
    displayName: 'Google Chrome - Personal',
    browserName: 'Google Chrome',
    profileName: 'Personal',
    supportedItems: [
      'history',
      'bookmarks',
      'cookies',
      'passwords',
      'autofill',
    ],
    recommendedItems: ['history', 'bookmarks', 'cookies', 'passwords'],
  },
  {
    id: 'edge-default',
    displayName: 'Microsoft Edge - Default',
    browserName: 'Microsoft Edge',
    profileName: 'Default',
    supportedItems: ['history', 'bookmarks', 'cookies', 'passwords'],
    recommendedItems: ['history', 'bookmarks', 'cookies'],
  },
]

export const DEFAULT_BROWSEROS_IMPORT_SOURCE_ID =
  MOCK_BROWSEROS_IMPORT_SOURCES[0]?.id ?? ''

const IMPORT_ITEM_LABELS: Record<string, string> = {
  history: 'History',
  bookmarks: 'Bookmarks',
  cookies: 'Cookies',
  passwords: 'Passwords',
  searchEngines: 'Search engines',
  autofill: 'Autofill',
  extensions: 'Extensions',
}

function humanizeImportItem(item: string): string {
  const label = item
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[-_]+/g, ' ')
    .trim()
  if (!label) return 'Unknown data'
  return label.charAt(0).toUpperCase() + label.slice(1)
}

export function importItemLabel(item: string): string {
  return IMPORT_ITEM_LABELS[item] ?? humanizeImportItem(item)
}

export function importItemListLabel(items: readonly string[]): string {
  if (items.length === 0) return 'No supported data'
  return items.map(importItemLabel).join(', ')
}

export function selectableItemsForSource(
  source: BrowserOSImportSource,
): BrowserOSImportItem[] {
  return [
    ...(source.recommendedItems.length
      ? source.recommendedItems
      : source.supportedItems),
  ]
}

export function sanitizeImportSelection(
  source: BrowserOSImportSource,
  items: readonly BrowserOSImportItem[],
): BrowserOSImportItem[] {
  const selectedItems = new Set(items)
  const emittedItems = new Set<BrowserOSImportItem>()
  return source.supportedItems.filter((item) => {
    if (!selectedItems.has(item) || emittedItems.has(item)) return false
    emittedItems.add(item)
    return true
  })
}

export function selectedSourceById(
  sources: readonly BrowserOSImportSource[],
  sourceId: string,
): BrowserOSImportSource | undefined {
  return sources.find((source) => source.id === sourceId)
}

export interface ImportSourceSelectionChange {
  selectedSourceId: string
  selectedItems: BrowserOSImportItem[]
}

export function importSourceSelectionChangeFor(
  sources: readonly BrowserOSImportSource[],
  currentSourceId: string,
): ImportSourceSelectionChange | null {
  if (sources.length === 0) {
    return { selectedSourceId: '', selectedItems: [] }
  }
  if (selectedSourceById(sources, currentSourceId)) return null
  const nextSource = sources[0]
  return {
    selectedSourceId: nextSource.id,
    selectedItems: selectableItemsForSource(nextSource),
  }
}

export function startImportRequestFor(
  source: BrowserOSImportSource,
  items?: readonly BrowserOSImportItem[],
): BrowserOSStartImportRequest | null {
  const importItems =
    items === undefined
      ? selectableItemsForSource(source)
      : sanitizeImportSelection(source, items)
  if (importItems.length === 0) return null
  return {
    sourceId: source.id,
    items: importItems,
  }
}

export function completedImportItemCount(
  progress: BrowserOSImportProgress | undefined,
): number {
  return progress?.completedItems.length ?? 0
}

export function importProgressTotal(
  selectedItemCount: number,
  progress: BrowserOSImportProgress | undefined,
): number {
  return progress?.totalItems ?? selectedItemCount
}

export const STARTER_PROMPTS: readonly string[] = [
  'Find me a coffee shop within walking distance and save it to my Maps.',
  'Apply for the SF visa for me, you have my passport scan in iCloud.',
]
