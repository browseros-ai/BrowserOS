import { describe, expect, it } from 'bun:test'
import {
  firstNonBlankLine,
  formatLocalDate,
  formatTokens,
  ROW_BAR_COUNT,
  truncate,
} from './agent-row.helpers'

describe('formatTokens', () => {
  it('renders zero / NaN as "0"', () => {
    expect(formatTokens(0)).toBe('0')
    expect(formatTokens(Number.NaN)).toBe('0')
  })

  it('renders sub-1K as integer', () => {
    expect(formatTokens(142)).toBe('142')
  })

  it('renders K with one decimal under 10', () => {
    expect(formatTokens(8_400)).toBe('8.4K')
  })

  it('drops the decimal at >=10K', () => {
    expect(formatTokens(120_000)).toBe('120K')
  })

  it('renders M with one decimal under 10', () => {
    expect(formatTokens(1_200_000)).toBe('1.2M')
  })
})

describe('firstNonBlankLine', () => {
  it('returns the first non-blank line', () => {
    expect(firstNonBlankLine('\n\nhello\nworld')).toBe('hello')
  })

  it('skips USER_QUERY envelope tags', () => {
    expect(firstNonBlankLine('<USER_QUERY>\nfix tests\n</USER_QUERY>')).toBe(
      'fix tests',
    )
  })

  it('falls back to the trimmed input when nothing matches', () => {
    expect(firstNonBlankLine('   single   ')).toBe('single')
  })
})

describe('truncate', () => {
  it('returns input unchanged when within limit', () => {
    expect(truncate('hello', 10)).toBe('hello')
  })

  it('appends an ellipsis when over limit', () => {
    expect(truncate('hello world', 6)).toBe('hello…')
  })
})

describe('formatLocalDate', () => {
  const today = new Date('2026-04-30T12:00:00Z')

  it('labels today and yesterday explicitly', () => {
    expect(formatLocalDate(ROW_BAR_COUNT - 1, today)).toBe('today')
    expect(formatLocalDate(ROW_BAR_COUNT - 2, today)).toBe('yesterday')
  })

  it('returns a "Mon D" format for older days', () => {
    const label = formatLocalDate(0, today)
    // "Apr 17" or "Apr 17," depending on locale; just assert it
    // contains a month abbreviation and a day number.
    expect(label).toMatch(/[A-Za-z]+ \d+/)
  })
})
