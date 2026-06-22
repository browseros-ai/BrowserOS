import { describe, expect, it } from 'bun:test'
import {
  aggregateLevel,
  chipTextFor,
  haloAmplitudeFor,
  personaStateFor,
  showsDots,
  synthesizedSpeakingEnvelope,
} from './voice-mode.helpers'

describe('chipTextFor', () => {
  it('returns Listening over agent when responding + isBargingIn', () => {
    expect(chipTextFor('responding', true, null)).toBe('Listening over agent')
  })

  it('returns Speaking when responding without barge-in', () => {
    expect(chipTextFor('responding', false, null)).toBe('Speaking')
  })

  it('returns the error message when in error state', () => {
    expect(chipTextFor('error', false, 'Microphone denied')).toBe(
      'Microphone denied',
    )
  })

  it('falls back to generic error when state is error but message is null', () => {
    expect(chipTextFor('error', false, null)).toBe('Something went wrong')
  })

  it('maps each non-terminal state to a single-word chip', () => {
    expect(chipTextFor('listening', false, null)).toBe('Listening')
    expect(chipTextFor('capturing', false, null)).toBe('Capturing')
    expect(chipTextFor('transcribing', false, null)).toBe('Transcribing')
  })

  it('returns empty string for closed and idle so the chip slot collapses', () => {
    expect(chipTextFor('closed', false, null)).toBe('')
    expect(chipTextFor('idle', false, null)).toBe('')
  })
})

describe('showsDots', () => {
  it('is true only for capturing and transcribing', () => {
    expect(showsDots('capturing')).toBe(true)
    expect(showsDots('transcribing')).toBe(true)
    expect(showsDots('listening')).toBe(false)
    expect(showsDots('responding')).toBe(false)
    expect(showsDots('idle')).toBe(false)
    expect(showsDots('closed')).toBe(false)
    expect(showsDots('error')).toBe(false)
  })
})

describe('personaStateFor', () => {
  it('returns asleep during the warm-up window regardless of state', () => {
    expect(personaStateFor({ state: 'listening', isWarmingUp: true })).toBe(
      'asleep',
    )
    expect(personaStateFor({ state: 'capturing', isWarmingUp: true })).toBe(
      'asleep',
    )
  })

  it('maps the voice state to a persona presentational state', () => {
    expect(personaStateFor({ state: 'responding', isWarmingUp: false })).toBe(
      'speaking',
    )
    expect(personaStateFor({ state: 'transcribing', isWarmingUp: false })).toBe(
      'thinking',
    )
    expect(personaStateFor({ state: 'listening', isWarmingUp: false })).toBe(
      'listening',
    )
    expect(personaStateFor({ state: 'capturing', isWarmingUp: false })).toBe(
      'listening',
    )
    expect(personaStateFor({ state: 'idle', isWarmingUp: false })).toBe('idle')
    expect(personaStateFor({ state: 'closed', isWarmingUp: false })).toBe(
      'idle',
    )
    expect(personaStateFor({ state: 'error', isWarmingUp: false })).toBe('idle')
  })
})

describe('aggregateLevel', () => {
  it('averages the bands', () => {
    expect(aggregateLevel([10, 20, 30, 40, 50])).toBe(30)
  })

  it('returns 0 for an empty array', () => {
    expect(aggregateLevel([])).toBe(0)
  })
})

describe('synthesizedSpeakingEnvelope', () => {
  it('stays between 0.3 and 0.6 for any input', () => {
    for (let t = 0; t < 10000; t += 17) {
      const v = synthesizedSpeakingEnvelope(t)
      expect(v).toBeGreaterThanOrEqual(0.3)
      expect(v).toBeLessThanOrEqual(0.6)
    }
  })
})

describe('haloAmplitudeFor', () => {
  it('returns aggregate-of-levels normalized to 0..1 while capturing', () => {
    const a = haloAmplitudeFor(
      { state: 'capturing', audioLevels: [50, 50, 50, 50, 50] },
      0,
    )
    expect(a).toBe(0.5)
  })

  it('returns the synthesized envelope while responding', () => {
    const a = haloAmplitudeFor(
      { state: 'responding', audioLevels: [0, 0, 0, 0, 0] },
      0,
    )
    expect(a).toBe(synthesizedSpeakingEnvelope(0))
  })

  it('returns 0 in other states regardless of levels', () => {
    expect(
      haloAmplitudeFor(
        { state: 'listening', audioLevels: [99, 99, 99, 99, 99] },
        0,
      ),
    ).toBe(0)
    expect(
      haloAmplitudeFor({ state: 'idle', audioLevels: [50, 50, 50, 50, 50] }, 0),
    ).toBe(0)
  })
})
