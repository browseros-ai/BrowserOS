/**
 * Find in output: the matches of a query in the shown lines, stepping between them, and cutting
 * a toned line where a match starts and ends so the match can be marked without losing tones.
 */

import type { Line, Tone } from './text-lines'

export interface Match {
  line: number
  start: number
  end: number
}

function escapeRegExp(query: string): string {
  return query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** Every match of `query` (case-insensitive, not overlapping), in reading order. */
export function findMatches(lines: string[], query: string): Match[] {
  if (query === '') return []
  // A regex keeps indices on the original text; lowercasing first can change its length.
  const pattern = new RegExp(escapeRegExp(query), 'gi')
  const matches: Match[] = []
  lines.forEach((text, line) => {
    for (const found of text.matchAll(pattern)) {
      const start = found.index ?? 0
      matches.push({ line, start, end: start + found[0].length })
    }
  })
  return matches
}

/** The next (or previous) match after `index`, wrapping at either end; -1 when there are none. */
export function stepMatch(
  index: number,
  total: number,
  direction: 1 | -1,
): number {
  if (total === 0) return -1
  if (index < 0) return direction === 1 ? 0 : total - 1
  return (index + direction + total) % total
}

export type Mark = 'none' | 'match' | 'current'

export interface MarkedSegment {
  text: string
  tone: Tone
  mark: Mark
}

/** The line's segments cut at each match's edges, each piece marked as in a match or not. */
export function markLine(
  line: Line,
  ranges: { start: number; end: number; current: boolean }[],
): MarkedSegment[] {
  const marked: MarkedSegment[] = []
  let offset = 0
  for (const segment of line) {
    const segmentEnd = offset + segment.text.length
    let at = offset
    for (const range of ranges) {
      if (range.end <= at || range.start >= segmentEnd) continue
      const start = Math.max(range.start, at)
      const end = Math.min(range.end, segmentEnd)
      if (start > at) {
        marked.push({
          text: segment.text.slice(at - offset, start - offset),
          tone: segment.tone,
          mark: 'none',
        })
      }
      marked.push({
        text: segment.text.slice(start - offset, end - offset),
        tone: segment.tone,
        mark: range.current ? 'current' : 'match',
      })
      at = end
    }
    if (at < segmentEnd) {
      marked.push({
        text: segment.text.slice(at - offset),
        tone: segment.tone,
        mark: 'none',
      })
    }
    offset = segmentEnd
  }
  return marked
}
