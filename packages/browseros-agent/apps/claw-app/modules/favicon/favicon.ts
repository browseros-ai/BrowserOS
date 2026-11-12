/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  browserFaviconUrl,
  duckDuckGoFaviconUrl,
  googleFaviconUrl,
  NEVER_VISITED_PAGE,
  siteHost,
} from './favicon.helpers'

const defaultIcons = new Map<number, Promise<Uint8Array | null>>()

async function bytes(url: string): Promise<Uint8Array | null> {
  try {
    const response = await fetch(url)
    return response.ok ? new Uint8Array(await response.arrayBuffer()) : null
  } catch {
    return null
  }
}

function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((byte, index) => byte === b[index])
}

/**
 * True when the browser's cache really holds this site's icon. The `_favicon`
 * endpoint never fails on a miss; it answers with the browser's default icon,
 * so a miss is spotted by comparing against the icon of a never-visited page.
 */
async function isCachedIcon(url: string, size: number): Promise<boolean> {
  const defaultUrl = browserFaviconUrl(NEVER_VISITED_PAGE, size)
  if (!defaultUrl) return false
  let defaultIcon = defaultIcons.get(size)
  if (!defaultIcon) {
    defaultIcon = bytes(defaultUrl)
    defaultIcons.set(size, defaultIcon)
  }
  const [icon, fallback] = await Promise.all([bytes(url), defaultIcon])
  if (!icon) return false
  return !fallback || !sameBytes(icon, fallback)
}

/** True inside the extension, where the browser's favicon cache can be read. */
export function canReadBrowserFavicons(): boolean {
  return browserFaviconUrl(NEVER_VISITED_PAGE, 16) !== null
}

/** DuckDuckGo then Google. They only ever see the host. */
export function webFaviconSources(site: string, size: number): string[] {
  const host = siteHost(site)
  return host ? [duckDuckGoFaviconUrl(host), googleFaviconUrl(host, size)] : []
}

/**
 * Where to load a site's icon from, best first: the browser's favicon cache,
 * then DuckDuckGo, then Google.
 */
export async function faviconSources(
  site: string,
  size: number,
): Promise<string[]> {
  const host = siteHost(site)
  if (!host) return []
  const web = webFaviconSources(host, size)
  const cached = browserFaviconUrl(`https://${host}`, size)
  if (!cached) return web
  return (await isCachedIcon(cached, size)) ? [cached, ...web] : web
}

/** Forgets the browser's default icon; for tests. */
export function resetFaviconCache(): void {
  defaultIcons.clear()
}
