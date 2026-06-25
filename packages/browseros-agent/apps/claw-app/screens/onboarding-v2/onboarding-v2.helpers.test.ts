import { describe, expect, it } from 'bun:test'
import {
  CHROME_PROFILES,
  profilesByIds,
  STARTER_PROMPTS,
  sumLoginsFor,
  sumSitesFor,
} from './onboarding-v2.helpers'

describe('CHROME_PROFILES fixture', () => {
  it('ships three profiles with stable ids', () => {
    expect(CHROME_PROFILES.map((p) => p.id)).toEqual([
      'work',
      'personal',
      'testing',
    ])
  })

  it('uses placeholder example.com emails (no real identifiers)', () => {
    for (const p of CHROME_PROFILES) {
      expect(p.email).toContain('example.com')
    }
  })
})

describe('sumSitesFor / sumLoginsFor', () => {
  it('sums the default selection (work + personal)', () => {
    expect(sumSitesFor(['work', 'personal'])).toBe(47)
    expect(sumLoginsFor(['work', 'personal'])).toBe(12)
  })

  it('returns 0 when the selection is empty', () => {
    expect(sumSitesFor([])).toBe(0)
    expect(sumLoginsFor([])).toBe(0)
  })

  it('handles single-profile selections', () => {
    expect(sumSitesFor(['testing'])).toBe(8)
    expect(sumLoginsFor(['testing'])).toBe(2)
  })
})

describe('profilesByIds', () => {
  it('returns profile records for the given ids, preserving fixture order', () => {
    const result = profilesByIds(['personal', 'work'])
    expect(result.map((p) => p.id)).toEqual(['work', 'personal'])
  })
})

describe('STARTER_PROMPTS', () => {
  it('ships at least two suggestions for the Ready step', () => {
    expect(STARTER_PROMPTS.length).toBeGreaterThanOrEqual(2)
  })
})
