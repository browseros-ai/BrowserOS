import { describe, expect, it } from 'bun:test'
import { importPhaseFor } from './Onboarding'

describe('importPhaseFor', () => {
  it('maps the Chromium importer status to a screen phase', () => {
    expect(importPhaseFor('importing')).toBe('importing')
    expect(importPhaseFor('failed')).toBe('failed')
    expect(importPhaseFor('succeeded')).toBe('imported')
  })

  it('treats every pre-import status as the picker', () => {
    expect(importPhaseFor('idle')).toBe('picker')
    expect(importPhaseFor('detecting')).toBe('picker')
    expect(importPhaseFor('ready')).toBe('picker')
  })
})
