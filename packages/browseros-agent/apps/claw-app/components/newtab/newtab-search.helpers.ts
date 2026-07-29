const DEFAULT_SEARCH_URL = 'https://www.google.com/search?q='

export type OmniIntent =
  | { kind: 'navigate'; url: string }
  | { kind: 'search'; text: string }

/**
 * A scheme like `https://` or a whitespace-free token carrying a dot reads as
 * a URL; anything else is a web search. A leading or trailing dot is not a
 * host, so it stays a search.
 */
export function looksLikeUrl(text: string): boolean {
  const trimmed = text.trim()
  if (trimmed.length === 0) return false
  if (/\s/.test(trimmed)) return false
  if (/^[a-z][\w+.-]*:\/\//i.test(trimmed)) return true
  if (trimmed === 'localhost' || /^localhost[:/]/.test(trimmed)) return true
  if (trimmed.startsWith('.') || trimmed.endsWith('.')) return false
  return trimmed.includes('.')
}

/** Prepends `https://` when the text carries no scheme. */
export function normalizeUrl(text: string): string {
  const trimmed = text.trim()
  if (/^[a-z][\w+.-]*:\/\//i.test(trimmed)) return trimmed
  return `https://${trimmed}`
}

/** Pure decision layer: a URL navigates, everything else searches. */
export function resolveOmni(text: string): OmniIntent | null {
  const trimmed = text.trim()
  if (trimmed.length === 0) return null
  if (looksLikeUrl(trimmed))
    return { kind: 'navigate', url: normalizeUrl(trimmed) }
  return { kind: 'search', text: trimmed }
}

/**
 * Routes the omnibox on submit. URLs load in the current tab. Searches prefer
 * the browser's configured default engine via `chrome.search.query`, which
 * keeps the query on-device, and fall back to a plain search URL when the API
 * is unavailable. The current tab is used because this page is the tab.
 */
export function submitOmni(text: string): void {
  const intent = resolveOmni(text)
  if (intent === null) return
  if (intent.kind === 'navigate') {
    window.location.assign(intent.url)
    return
  }
  const search = globalThis.chrome?.search
  if (typeof search?.query === 'function') {
    search.query({ text: intent.text, disposition: 'CURRENT_TAB' })
    return
  }
  window.location.assign(
    `${DEFAULT_SEARCH_URL}${encodeURIComponent(intent.text)}`,
  )
}
