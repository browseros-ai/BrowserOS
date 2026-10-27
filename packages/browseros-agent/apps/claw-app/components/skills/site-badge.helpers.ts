/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * The browser's own copy of a site's icon, read from its favicon cache through
 * the extension `_favicon` endpoint, so it needs no network and no third-party
 * service. Null outside the extension, where that endpoint does not exist.
 */
export function faviconUrl(site: string, size = 32): string | null {
  if (typeof chrome === 'undefined' || !chrome.runtime?.id) return null
  const pageUrl = /^https?:\/\//i.test(site) ? site : `https://${site}`
  const url = new URL(chrome.runtime.getURL('/_favicon/'))
  url.searchParams.set('pageUrl', pageUrl)
  url.searchParams.set('size', String(size))
  return url.toString()
}
