import { describe, expect, it } from 'bun:test'
import {
  aggregateLevel,
  chipTextFor,
  haloAmplitudeFor,
  orbStateFor,
  showsDots,
} from './voice-mode.helpers'

describe('chipTextFor', () => {
  it('returns the agent-working chip when responding', () => {
    expect(chipTextFor('responding', null)).toBe('BrowserOS Agent is working')
  })

  it('returns the agent-working chip during barge_in_pending', () => {
    expect(chipTextFor('barge_in_pending', null)).toBe(
      'BrowserOS Agent is working',
    )
  })

  it('returns the error message when in error state', () => {
    expect(chipTextFor('error', 'Microphone denied')).toBe('Microphone denied')
  })

  it('falls back to generic error when state is error but message is null', () => {
    expect(chipTextFor('error', null)).toBe('Something went wrong')
  })

  it('maps each non-terminal state to a single-word chip', () => {
    expect(chipTextFor('listening', null)).toBe('Listening')
    expect(chipTextFor('capturing', null)).toBe('Capturing')
    expect(chipTextFor('transcribing', null)).toBe('Transcribing')
  })

  it('returns empty string for closed and idle so the chip slot collapses', () => {
    expect(chipTextFor('closed', null)).toBe('')
    expect(chipTextFor('idle', null)).toBe('')
  })
})

describe('showsDots', () => {
  it('is true only for capturing and transcribing', () => {
    expect(showsDots('capturing')).toBe(true)
    expect(showsDots('transcribing')).toBe(true)
    expect(showsDots('listening')).toBe(false)
    expect(showsDots('responding')).toBe(false)
    expect(showsDots('barge_in_pending')).toBe(false)
    expect(showsDots('idle')).toBe(false)
    expect(showsDots('closed')).toBe(false)
    expect(showsDots('error')).toBe(false)
  })
})

describe('orbStateFor', () => {
  it('returns idle during the warm-up window regardless of state', () => {
    expect(orbStateFor({ state: 'listening', isWarmingUp: true })).toBe('idle')
    expect(orbStateFor({ state: 'capturing', isWarmingUp: true })).toBe('idle')
  })

  it('maps responding and transcribing to speaking', () => {
    expect(orbStateFor({ state: 'responding', isWarmingUp: false })).toBe(
      'speaking',
    )
    expect(orbStateFor({ state: 'transcribing', isWarmingUp: false })).toBe(
      'speaking',
    )
  })

  it('maps listening and capturing to listening', () => {
    expect(orbStateFor({ state: 'listening', isWarmingUp: false })).toBe(
      'listening',
    )
    expect(orbStateFor({ state: 'capturing', isWarmingUp: false })).toBe(
      'listening',
    )
  })

  it('maps barge_in_pending to listening', () => {
    expect(orbStateFor({ state: 'barge_in_pending', isWarmingUp: false })).toBe(
      'listening',
    )
  })

  it('falls back to idle for closed, idle, and error', () => {
    expect(orbStateFor({ state: 'idle', isWarmingUp: false })).toBe('idle')
    expect(orbStateFor({ state: 'closed', isWarmingUp: false })).toBe('idle')
    expect(orbStateFor({ state: 'error', isWarmingUp: false })).toBe('idle')
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

describe('haloAmplitudeFor', () => {
  it('returns aggregate-of-levels normalized to 0..1 while capturing', () => {
    const a = haloAmplitudeFor({
      state: 'capturing',
      audioLevels: [50, 50, 50, 50, 50],
    })
    expect(a).toBe(0.5)
  })

  it('returns 0 while responding (CSS pulse handles the visual)', () => {
    expect(
      haloAmplitudeFor({
        state: 'responding',
        audioLevels: [0, 0, 0, 0, 0],
      }),
    ).toBe(0)
  })

  it('returns 0 in other states regardless of levels', () => {
    expect(
      haloAmplitudeFor({
        state: 'listening',
        audioLevels: [99, 99, 99, 99, 99],
      }),
    ).toBe(0)
    expect(
      haloAmplitudeFor({
        state: 'idle',
        audioLevels: [50, 50, 50, 50, 50],
      }),
    ).toBe(0)
  })
})
