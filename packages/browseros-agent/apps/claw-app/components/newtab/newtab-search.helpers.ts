const SCHEME_RE = /^[a-z][a-z0-9+.-]*:\/\//i
const KNOWN_SCHEME_RE = /^(chrome|about|file|edge|view-source):/i
const DEFAULT_SEARCH_URL = 'https://www.google.com/search?q='

/**
 * True when the text reads as a navigable location rather than a query: an
 * explicit scheme, or a dotted token with no whitespace (for example
 * `example.com` or `docs.browseros.com/x`).
 */
export function looksLikeUrl(text: string): boolean {
  const trimmed = text.trim()
  if (trimmed.length === 0) return false
  if (SCHEME_RE.test(trimmed) || KNOWN_SCHEME_RE.test(trimmed)) return true
  if (/\s/.test(trimmed)) return false
  return trimmed.includes('.')
}

/** Resolves a URL-like token to a full URL, defaulting scheme-less input to https. */
export function toUrl(text: string): string {
  const trimmed = text.trim()
  if (SCHEME_RE.test(trimmed) || KNOWN_SCHEME_RE.test(trimmed)) return trimmed
  return `https://${trimmed}`
}

/**
 * Acts on omnibox input in the current tab: navigate a URL, otherwise run a
 * web search through the user's default engine when available, falling back
 * to a default search URL.
 */
export function submitOmni(text: string): void {
  const trimmed = text.trim()
  if (trimmed.length === 0) return
  if (looksLikeUrl(trimmed)) {
    window.location.assign(toUrl(trimmed))
    return
  }
  if (typeof chrome !== 'undefined' && chrome.search?.query) {
    chrome.search.query({ text: trimmed, disposition: 'CURRENT_TAB' })
    return
  }
  window.location.assign(`${DEFAULT_SEARCH_URL}${encodeURIComponent(trimmed)}`)
}
