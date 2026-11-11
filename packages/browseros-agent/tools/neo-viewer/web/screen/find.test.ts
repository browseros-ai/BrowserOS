import { describe, expect, it } from 'bun:test'
import { findMatches, markLine, stepMatch } from './find'
import type { Line } from './text-lines'

describe('find in output', () => {
  const lines = [
    '- textbox "Full name" [ref=e5]',
    '- combobox "Country" [ref=e7]:',
    '  - option "United States"',
    'no match here',
  ]

  it('finds every match, ignoring case, in reading order', () => {
    expect(findMatches(lines, 'REF=E')).toEqual([
      { line: 0, start: 23, end: 28 },
      { line: 1, start: 22, end: 27 },
    ])
    expect(findMatches(lines, 'Option')).toEqual([
      { line: 2, start: 4, end: 10 },
    ])
    expect(findMatches(lines, '')).toEqual([])
  })

  it('treats the query as text, not a pattern', () => {
    expect(findMatches(['a.b', 'axb', '[ref=e5]'], 'a.b')).toEqual([
      { line: 0, start: 0, end: 3 },
    ])
    expect(findMatches(['[ref=e5]'], '[ref=')).toHaveLength(1)
  })

  it('steps forward and back through the matches, wrapping at the ends', () => {
    expect(stepMatch(0, 3, 1)).toBe(1)
    expect(stepMatch(2, 3, 1)).toBe(0)
    expect(stepMatch(0, 3, -1)).toBe(2)
    expect(stepMatch(-1, 3, -1)).toBe(2)
    expect(stepMatch(0, 0, 1)).toBe(-1)
  })

  it('cuts a toned line at the match edges without losing the tones', () => {
    const line: Line = [
      { text: '- textbox ', tone: 'plain' },
      { text: '"Full name"', tone: 'string' },
      { text: ' ', tone: 'plain' },
      { text: '[ref=e5]', tone: 'ref' },
    ]
    // "box \"Full" runs across the plain and string segments.
    const marked = markLine(line, [
      { start: 6, end: 15, current: true },
      { start: 27, end: 30, current: false },
    ])
    expect(marked).toEqual([
      { text: '- text', tone: 'plain', mark: 'none' },
      { text: 'box ', tone: 'plain', mark: 'current' },
      { text: '"Full', tone: 'string', mark: 'current' },
      { text: ' name"', tone: 'string', mark: 'none' },
      { text: ' ', tone: 'plain', mark: 'none' },
      { text: '[ref=', tone: 'ref', mark: 'none' },
      { text: 'e5]', tone: 'ref', mark: 'match' },
    ])
    expect(marked.map((piece) => piece.text).join('')).toBe(
      '- textbox "Full name" [ref=e5]',
    )
  })
})
