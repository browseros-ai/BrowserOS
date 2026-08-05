/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Persisted collapse state for the cockpit video rail, in chrome.storage.local
 * via WXT. Defaults to expanded; once the reader collapses it, it stays
 * collapsed across new tabs until they expand it again.
 *
 * Defined lazily: `defineItem` touches chrome.storage on first use, so building
 * it on demand keeps importing this module safe outside the extension (e.g.
 * under test) where `browser` is undefined.
 */

import { storage } from '@wxt-dev/storage'

let item: ReturnType<typeof define> | undefined

function define() {
  return storage.defineItem<boolean>('local:cockpit.videos.collapsed', {
    fallback: false,
  })
}

export function cockpitVideosCollapsedStorage() {
  if (!item) item = define()
  return item
}
