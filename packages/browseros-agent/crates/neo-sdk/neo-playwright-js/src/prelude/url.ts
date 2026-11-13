/** URL comparisons use the real realm globals installed by neo-sdk before this prelude. */
import { globToRegexPattern } from './vendored'

/** Playwright normalizes absolute URLs, and leaves a non-URL string unchanged. */
export function normalizeUrl(text: string): string {
  return URL.parse(text)?.href ?? text
}

/** Playwright's glob test, for URLs the prelude already holds (`page.frame({ url })`). */
export function globMatches(glob: string, url: string): boolean {
  if (glob === '') return true
  return new RegExp(globToRegexPattern(glob)).test(url)
}
