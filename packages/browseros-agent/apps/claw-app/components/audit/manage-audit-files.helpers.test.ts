import { describe, expect, it } from 'bun:test'
import {
  choiceFromRetention,
  cleanupConfirmText,
  formatBytes,
  retentionRequest,
} from './manage-audit-files.helpers'

describe('formatBytes', () => {
  it('renders MB below a gigabyte and GB above', () => {
    expect(formatBytes(0)).toBe('0 MB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB')
    expect(formatBytes(340 * 1024 * 1024)).toBe('340 MB')
    expect(formatBytes(1.5 * 1024 ** 3)).toBe('1.50 GB')
  })

  it('never renders a negative or NaN size', () => {
    expect(formatBytes(-1)).toBe('0 MB')
    expect(formatBytes(Number.NaN)).toBe('0 MB')
  })
})

describe('retention <-> choice mapping', () => {
  it('round-trips the presets and never', () => {
    expect(choiceFromRetention({ mode: 'keepForever' }).choice).toBe('never')
    expect(
      choiceFromRetention({ mode: 'deleteAfterDays', days: 7 }).choice,
    ).toBe('7')
    expect(
      choiceFromRetention({ mode: 'deleteAfterDays', days: 30 }).choice,
    ).toBe('30')
    const custom = choiceFromRetention({ mode: 'deleteAfterDays', days: 45 })
    expect(custom.choice).toBe('custom')
    expect(custom.days).toBe(45)
  })

  it('builds valid request bodies', () => {
    expect(retentionRequest('never', 30)).toEqual({ mode: 'keepForever' })
    expect(retentionRequest('7', 30)).toEqual({
      mode: 'deleteAfterDays',
      days: 7,
    })
    expect(retentionRequest('custom', 45)).toEqual({
      mode: 'deleteAfterDays',
      days: 45,
    })
    // custom days is clamped to at least 1
    expect(retentionRequest('custom', 0)).toEqual({
      mode: 'deleteAfterDays',
      days: 1,
    })
  })
})

describe('cleanupConfirmText', () => {
  it('mentions the window for deleteAfterDays and orphans for never', () => {
    expect(cleanupConfirmText({ mode: 'deleteAfterDays', days: 30 })).toContain(
      'older than 30 days',
    )
    expect(cleanupConfirmText({ mode: 'keepForever' })).toContain('Never')
  })
})
