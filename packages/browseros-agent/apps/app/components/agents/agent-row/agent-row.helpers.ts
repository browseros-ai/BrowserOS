/**
 * Pure formatters consumed by row sub-components. Kept distinct from
 * `agent-display.helpers.ts` (page-level helpers) so the row internals
 * have an obvious single home.
 */

const TOKEN_THRESHOLDS: Array<[number, string]> = [
  [1_000_000, 'M'],
  [1_000, 'K'],
]

/** `1.2M`, `820K`, `8.4K`, `142`, `0`. */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0'
  for (const [threshold, suffix] of TOKEN_THRESHOLDS) {
    if (n >= threshold) {
      const value = n / threshold
      const decimal = value < 10 ? value.toFixed(1) : value.toFixed(0)
      return `${decimal}${suffix}`
    }
  }
  return String(Math.round(n))
}

const USER_QUERY_OPEN = /^<USER_QUERY>$/i
const USER_QUERY_CLOSE = /^<\/USER_QUERY>$/i

/**
 * First non-blank line, with the BrowserOS user-system-prompt
 * `<USER_QUERY>` envelope tags stripped so previews don't show
 * structural noise.
 */
export function firstNonBlankLine(text: string): string {
  const lines = text.split('\n').map((line) => line.trim())
  for (const line of lines) {
    if (!line) continue
    if (USER_QUERY_OPEN.test(line) || USER_QUERY_CLOSE.test(line)) continue
    return line
  }
  return text.trim()
}

export function truncate(text: string, max: number): string {
  if (text.length <= max) return text
  return `${text.slice(0, max - 1).trimEnd()}…`
}

const SPARKLINE_DAYS = 14

/**
 * "today" / "yesterday" / "Apr 17" — given an index 0..13 from
 * oldest → newest. `today` defaults to `new Date()` so callers don't
 * have to thread a clock through.
 */
export function formatLocalDate(idx: number, today: Date = new Date()): string {
  if (idx === SPARKLINE_DAYS - 1) return 'today'
  if (idx === SPARKLINE_DAYS - 2) return 'yesterday'
  const offset = SPARKLINE_DAYS - 1 - idx
  const date = new Date(today)
  date.setDate(date.getDate() - offset)
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

export const ROW_BAR_COUNT = SPARKLINE_DAYS
