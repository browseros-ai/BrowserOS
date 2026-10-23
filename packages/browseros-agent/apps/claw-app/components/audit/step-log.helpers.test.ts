import { describe, expect, it } from 'bun:test'
import { formatStepDuration, preciseOffset } from './step-log.helpers'

describe('preciseOffset', () => {
  it('formats minutes, seconds and hundredths from the task start', () => {
    expect(preciseOffset(1_000, 1_000)).toBe('0:00.00')
    expect(preciseOffset(1_000 + 65_427, 1_000)).toBe('1:05.42')
    expect(preciseOffset(500, 1_000)).toBe('0:00.00')
  })
})

describe('formatStepDuration', () => {
  it('uses ms below a second, seconds above, and nothing when unknown', () => {
    expect(formatStepDuration(260)).toBe('260 ms')
    expect(formatStepDuration(1_240)).toBe('1.2 s')
    expect(formatStepDuration(undefined)).toBe('')
  })
})
