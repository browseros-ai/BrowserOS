/**
 * The page's view of `shared/code-format.ts`, cached by script: rows redraw every 250 ms while a
 * run is running, and parsing each script again each time would be waste. The code as sent is
 * the key; token counts stay computed on it (`RunSummary.codeSize`), never on the layout.
 */

import {
  type FormattedCode,
  firstStatement,
  formatCode,
} from '../../shared/code-format'

const KEPT = 200
const formatted = new Map<string, FormattedCode>()
const first = new Map<string, string>()

function cached<T>(cache: Map<string, T>, code: string, make: () => T): T {
  const hit = cache.get(code)
  if (hit !== undefined) return hit
  const value = make()
  cache.set(code, value)
  // Oldest first: Map keeps insertion order.
  if (cache.size > KEPT) cache.delete(cache.keys().next().value as string)
  return value
}

export function formattedCode(code: string): FormattedCode {
  return cached(formatted, code, () => formatCode(code))
}

export function firstStatementOf(code: string): string {
  return cached(first, code, () => firstStatement(code))
}
