/**
 * On-device favicon URL from the extension `_favicon` route, so nothing about
 * the user's top sites leaves the machine. Returns an empty string when the
 * runtime is unavailable (dev build, tests) so callers fall back to a glyph.
 */
export function faviconUrl(pageUrl: string, size = 32): string {
  const runtime = globalThis.chrome?.runtime
  if (typeof runtime?.getURL !== 'function') return ''
  const params = new URLSearchParams({ pageUrl, size: String(size) })
  return runtime.getURL(`/_favicon/?${params.toString()}`)
}
