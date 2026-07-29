export type OmniSearchAction =
  | { kind: 'url'; url: string }
  | { kind: 'search'; text: string }

const HAS_SCHEME = /^[a-z][a-z0-9+.-]*:\/\//i
const IS_LOCALHOST = /^localhost(:\d+)?(\/.*)?$/i

/** True when the text should open as a location rather than a web search. */
export function looksLikeUrl(raw: string): boolean {
  const text = raw.trim()
  if (text.length === 0 || /\s/.test(text)) return false
  if (HAS_SCHEME.test(text)) return true
  if (IS_LOCALHOST.test(text)) return true
  return text.includes('.')
}

/** Classifies omnibox input into a navigation or a web-search intent. */
export function resolveOmniSearch(raw: string): OmniSearchAction | null {
  const text = raw.trim()
  if (text.length === 0) return null
  if (looksLikeUrl(text)) {
    return {
      kind: 'url',
      url: HAS_SCHEME.test(text) ? text : `https://${text}`,
    }
  }
  return { kind: 'search', text }
}

/** Default-engine search URL used only when the search API is unavailable. */
export function defaultSearchUrl(text: string): string {
  return `https://www.google.com/search?q=${encodeURIComponent(text)}`
}

function navigateCurrentTab(url: string): void {
  window.location.assign(url)
}

/**
 * Runs the omnibox intent in the current tab. Web searches go through the
 * user's default engine via chrome.search so no engine is hardcoded and the
 * query never leaves the machine through this extension; the default-engine
 * URL is only a fallback when that API is missing.
 */
export function runOmniSearch(raw: string): void {
  const action = resolveOmniSearch(raw)
  if (!action) return
  if (action.kind === 'url') {
    navigateCurrentTab(action.url)
    return
  }
  if (
    typeof chrome !== 'undefined' &&
    typeof chrome.search?.query === 'function'
  ) {
    void chrome.search.query({
      text: action.text,
      disposition: 'CURRENT_TAB',
    })
    return
  }
  navigateCurrentTab(defaultSearchUrl(action.text))
}
