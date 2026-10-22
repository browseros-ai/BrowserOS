/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Import sources in the shapes Chromium sends ("Google Chrome - Work",
 * "Microsoft Edge - Default"). The bridge serves them when the page runs
 * outside Chromium (Vite dev, previews), and tests build states from them.
 */

import type { BrowserOSImportSource } from './browseros-onboarding-api'

export const MOCK_BROWSEROS_IMPORT_SOURCES: readonly BrowserOSImportSource[] = [
  {
    id: 'chrome-work',
    displayName: 'Google Chrome - Work',
    browserName: 'Google Chrome',
    profileName: 'Work',
    accountName: 'work@example.com',
    isManaged: true,
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
    accountName: 'personal@example.com',
    isManaged: false,
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
    accountName: '',
    isManaged: false,
    supportedItems: ['history', 'bookmarks', 'cookies', 'passwords'],
    recommendedItems: ['history', 'bookmarks', 'cookies'],
  },
]
