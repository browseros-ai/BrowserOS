/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Where a site's icon can come from. Pure URL builders, no side effects.
 */

/** The host of a site written as "x.com", "https://x.com/home" or "X.com"; null when there is none. */
export function siteHost(site: string): string | null {
  const trimmed = site.trim()
  if (!trimmed) return null
  try {
    const url = new URL(
      /^[a-z][a-z\d+.-]*:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`,
    )
    return url.hostname.toLowerCase() || null
  } catch {
    return null
  }
}

/**
 * The browser's own copy of the icon, read from its favicon cache through the
 * extension `_favicon` endpoint (needs the `favicon` permission). Null outside
 * the extension, where that endpoint does not exist.
 */
export function browserFaviconUrl(
  pageUrl: string,
  size: number,
): string | null {
  if (typeof chrome === 'undefined' || !chrome.runtime?.id) return null
  const url = new URL(chrome.runtime.getURL('/_favicon/'))
  url.searchParams.set('pageUrl', pageUrl)
  url.searchParams.set('size', String(size))
  return url.toString()
}

/** A page the browser can never have visited, so its icon is the browser's default. */
export const NEVER_VISITED_PAGE = 'https://neo-favicon-miss.invalid/'

export function duckDuckGoFaviconUrl(host: string): string {
  return `https://icons.duckduckgo.com/ip2/${host}.ico`
}

export function googleFaviconUrl(host: string, size: number): string {
  const url = new URL('https://www.google.com/s2/favicons')
  url.searchParams.set('sz', String(size))
  url.searchParams.set('domain_url', host)
  return url.toString()
}
