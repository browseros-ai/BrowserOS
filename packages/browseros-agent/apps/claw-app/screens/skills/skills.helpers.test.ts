import { describe, expect, it } from 'bun:test'
import {
  formatRelativeTime,
  formatTokens,
  neoName,
  parseSkillBody,
  skillCommand,
  successRate,
  tokenDeltaPercent,
} from './skills.helpers'

describe('skillCommand', () => {
  it('prefixes the name with a slash', () => {
    expect(skillCommand('inbox-sweep')).toBe('/inbox-sweep')
  })
})

describe('neoName', () => {
  it('namespaces a bare suffix under neo-', () => {
    expect(neoName('weather')).toBe('neo-weather')
  })

  it('does not double an over-typed neo- prefix', () => {
    expect(neoName('neo-weather')).toBe('neo-weather')
  })

  it('leaves an interior neo- untouched', () => {
    expect(neoName('weather-neo-check')).toBe('neo-weather-neo-check')
  })
})

describe('successRate', () => {
  it('is muted with no runs', () => {
    expect(successRate(0, 0)).toEqual({
      hasRuns: false,
      percent: 0,
      colorClass: 'text-ink-3',
    })
  })

  it('is green at or above 90 percent', () => {
    expect(successRate(2, 2)).toEqual({
      hasRuns: true,
      percent: 100,
      colorClass: 'text-green',
    })
    expect(successRate(9, 10).colorClass).toBe('text-green')
  })

  it('is amber between 70 and 89 percent', () => {
    expect(successRate(7, 10).colorClass).toBe('text-amber')
  })

  it('is red below 70 percent', () => {
    expect(successRate(1, 2).colorClass).toBe('text-red')
  })
})

describe('formatTokens', () => {
  it('leaves values under 1000 as-is', () => {
    expect(formatTokens(840)).toBe('840')
  })

  it('compacts thousands to one decimal', () => {
    expect(formatTokens(14600)).toBe('14.6k')
  })

  it('keeps the trailing decimal on a round thousand', () => {
    expect(formatTokens(23000)).toBe('23.0k')
  })
})

describe('tokenDeltaPercent', () => {
  it('is null when either side is missing', () => {
    expect(tokenDeltaPercent(undefined, 100)).toBeNull()
    expect(tokenDeltaPercent(100, undefined)).toBeNull()
  })

  it('is null when the first run measured zero tokens', () => {
    expect(tokenDeltaPercent(0, 100)).toBeNull()
  })

  it('is negative when the skill got cheaper', () => {
    expect(tokenDeltaPercent(23000, 14600)).toBe(-37)
  })

  it('is positive when the skill got pricier', () => {
    expect(tokenDeltaPercent(100, 150)).toBe(50)
  })
})

describe('parseSkillBody', () => {
  const body = [
    '---',
    'name: inbox-sweep',
    'description: "Check the inbox"',
    'tools: browseros-neo',
    '---',
    '',
    '## Steps',
    '1. Call the mark_skill_run tool with name: inbox-sweep so this run is recorded.',
    '2. Open the inbox',
    '3. Draft replies',
    '',
    '## Learned from past runs',
    '- Read the DOM snapshot, not screenshots',
    '- Leave drafts unsent',
  ].join('\n')

  it('extracts the user steps and drops the auto run-marking step', () => {
    expect(parseSkillBody(body).steps).toEqual([
      'Open the inbox',
      'Draft replies',
    ])
  })

  it('extracts the learned notes', () => {
    expect(parseSkillBody(body).learnedNotes).toEqual([
      'Read the DOM snapshot, not screenshots',
      'Leave drafts unsent',
    ])
  })

  it('returns empty arrays for a body with no sections', () => {
    expect(parseSkillBody('just prose')).toEqual({
      steps: [],
      learnedNotes: [],
    })
  })
})

describe('formatRelativeTime', () => {
  const now = 1_000_000_000_000

  it('reads "just now" under a minute', () => {
    expect(formatRelativeTime(now - 30_000, now)).toBe('just now')
  })

  it('reads minutes', () => {
    expect(formatRelativeTime(now - 5 * 60_000, now)).toBe('5m ago')
  })

  it('reads hours', () => {
    expect(formatRelativeTime(now - 3 * 3_600_000, now)).toBe('3h ago')
  })

  it('reads days', () => {
    expect(formatRelativeTime(now - 2 * 86_400_000, now)).toBe('2d ago')
  })

  it('never goes negative for a future timestamp', () => {
    expect(formatRelativeTime(now + 60_000, now)).toBe('just now')
  })
})
