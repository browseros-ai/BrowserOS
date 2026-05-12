/**
 * User preference extraction — vendored from pi-vcc extract/preferences.ts.
 */

import { clip, nonEmptyLines } from '../content'
import type { NormalizedBlock } from '../types'

const PREF_PATTERNS = [
  /\bprefer(?:s|red|ring)?\s+\w/i,
  /\bdon'?t want\b/i,
  /\balways (?:use|do|run|prefer|keep|make|format|write|add|set|put|prefix|start|include|append)\b/i,
  /\bnever (?:use|do|run|push|commit|write|ignore|add|set|put|remove|delete|include|deploy)\b/i,
  /\bplease (?:use|avoid|keep|make|don'?t|do not|format|write)\b/i,
  /\b(?:style|format|language|naming)\s*[:=]\s*\S/i,
]

export const extractPreferences = (blocks: NormalizedBlock[]): string[] => {
  const prefs: string[] = []
  const seen = new Set<string>()

  for (const b of blocks) {
    if (b.kind !== 'user') continue

    let perBlock = 0
    for (const line of nonEmptyLines(b.text)) {
      const trimmed = line.trim()
      if (!trimmed || trimmed.length < 5) continue
      if (trimmed.length > 200) continue
      if (trimmed.endsWith('?') || trimmed.includes('?...')) continue
      if (!PREF_PATTERNS.some((p) => p.test(trimmed))) continue

      const clipped = clip(trimmed, 200)
      const key = clipped.toLowerCase()
      if (seen.has(key)) continue
      seen.add(key)
      prefs.push(clipped)

      if (++perBlock >= 1) break
    }
  }

  return prefs.slice(0, 10)
}

export const dedupPreferencesAgainstGoals = (
  prefs: string[],
  goals: string[],
): string[] => {
  const norm = (s: string) => s.trim().toLowerCase()
  const goalSet = new Set(goals.map(norm))
  return prefs.filter((p) => !goalSet.has(norm(p)))
}
