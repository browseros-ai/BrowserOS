/** `T+` label for a screenshot's offset from task start. */
export function formatOffset(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)}s`
  const totalSec = Math.floor(seconds)
  const mins = Math.floor(totalSec / 60)
  const rem = totalSec % 60
  return `${mins}m${rem.toString().padStart(2, '0')}s`
}

/** Bare hostname (no `www.`) for a captured page URL, or `''` when unknown. */
export function hostOf(url: string | null): string {
  if (!url) return ''
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return ''
  }
}
