/**
 * Upstream's isomorphic/stringUtils as InjectedScript and the selector generator import it in
 * Neo's build (scripts/build.ts resolves `@isomorphic/stringUtils` here for those two files
 * only). It is the same module, with one change: the helpers through which those files cut or
 * quote page text for a preview or a locator hint mask the text first (page-mask.ts).
 *
 * - `trimStringWithEllipsis`: previewNode's cut of an element's attributes (500) and text (50).
 * - `escapeForTextSelector`, `escapeForAttributeSelector`, `quoteCSSAttributeValue`,
 *   `escapeRegExp`: the generator quotes and escapes every text, attribute, name and label it
 *   builds a candidate from, so the hint it picks never holds a value in a form Rust's mask
 *   would miss. A masked candidate matches nothing on the page, and the generator moves on to
 *   one that does.
 *
 * Everything else, and every other importer, stays upstream's, so selector matching never
 * sees a mask.
 */
import * as upstream from '../../../neo-playwright/third_party/playwright/packages/isomorphic/stringUtils'
import { maskPageText } from './page-mask'

export * from '../../../neo-playwright/third_party/playwright/packages/isomorphic/stringUtils'

export function trimStringWithEllipsis(input: string, cap: number): string {
  return upstream.trimStringWithEllipsis(maskPageText(input), cap)
}

export function escapeForTextSelector(
  text: string | RegExp,
  exact: boolean,
): string {
  // A RegExp here is the generator's own, built from escapeRegExp's masked output.
  return upstream.escapeForTextSelector(
    typeof text === 'string' ? maskPageText(text) : text,
    exact,
  )
}

export function escapeForAttributeSelector(
  value: string | RegExp,
  exact: boolean,
): string {
  return upstream.escapeForAttributeSelector(
    typeof value === 'string' ? maskPageText(value) : value,
    exact,
  )
}

export function quoteCSSAttributeValue(text: string): string {
  return upstream.quoteCSSAttributeValue(maskPageText(text))
}

export function escapeRegExp(s: string): string {
  return upstream.escapeRegExp(maskPageText(s))
}
