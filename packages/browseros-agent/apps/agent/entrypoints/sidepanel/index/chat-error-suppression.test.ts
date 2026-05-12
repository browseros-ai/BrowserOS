import { describe, expect, it } from 'bun:test'
import {
  computeVisibleChatError,
  isStaleErrorMarkerStillCurrent,
} from './chat-error-suppression'

describe('computeVisibleChatError', () => {
  it('returns undefined when there is no chat error', () => {
    expect(
      computeVisibleChatError({ chatError: undefined, staleErrorMarker: null }),
    ).toBeUndefined()
  })

  it('returns the chat error when no marker is set', () => {
    const error = new Error('Credits exhausted')
    expect(
      computeVisibleChatError({ chatError: error, staleErrorMarker: null }),
    ).toBe(error)
  })

  it('suppresses the chat error when its reference matches the marker', () => {
    const error = new Error('Invalid Authentication')
    expect(
      computeVisibleChatError({ chatError: error, staleErrorMarker: error }),
    ).toBeUndefined()
  })

  it('shows a fresh error reference even when a different one is suppressed', () => {
    const oldError = new Error('Credits exhausted')
    const newError = new Error('Failed to parse JSON')
    expect(
      computeVisibleChatError({
        chatError: newError,
        staleErrorMarker: oldError,
      }),
    ).toBe(newError)
  })

  it('treats structurally identical errors as distinct references', () => {
    // Two Error objects with the same message are still different references —
    // the suppression key is identity, not message equality.
    const first = new Error('Credits exhausted')
    const second = new Error('Credits exhausted')
    expect(
      computeVisibleChatError({ chatError: second, staleErrorMarker: first }),
    ).toBe(second)
  })
})

describe('isStaleErrorMarkerStillCurrent', () => {
  it('is false when no marker is stored', () => {
    expect(isStaleErrorMarkerStillCurrent(undefined, null)).toBe(false)
    expect(isStaleErrorMarkerStillCurrent(new Error('x'), null)).toBe(false)
  })

  it('is false when the marker exists but the live error has cleared', () => {
    const marker = new Error('Daily limit reached')
    expect(isStaleErrorMarkerStillCurrent(undefined, marker)).toBe(false)
  })

  it('is true when the live error still references the marker', () => {
    const marker = new Error('Daily limit reached')
    expect(isStaleErrorMarkerStillCurrent(marker, marker)).toBe(true)
  })

  it('is false when the live error is a different reference', () => {
    const marker = new Error('Daily limit reached')
    const live = new Error('Daily limit reached')
    expect(isStaleErrorMarkerStillCurrent(live, marker)).toBe(false)
  })
})
