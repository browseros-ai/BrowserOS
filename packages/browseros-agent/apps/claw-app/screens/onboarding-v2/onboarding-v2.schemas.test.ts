import { describe, expect, it } from 'bun:test'
import {
  onboardingFormDefaults,
  onboardingFormSchema,
} from './onboarding-v2.schemas'

describe('onboardingFormSchema', () => {
  it('accepts the default values', () => {
    const parsed = onboardingFormSchema.parse(onboardingFormDefaults)
    expect(parsed.selectedProfileIds).toEqual(['work', 'personal'])
  })

  it('rejects an empty selection with a helpful message', () => {
    const result = onboardingFormSchema.safeParse({ selectedProfileIds: [] })
    expect(result.success).toBe(false)
    if (!result.success) {
      expect(result.error.issues[0]?.message).toBe(
        'Pick at least one profile to import.',
      )
    }
  })

  it('rejects unknown profile ids', () => {
    const result = onboardingFormSchema.safeParse({
      selectedProfileIds: ['ghost'],
    })
    expect(result.success).toBe(false)
  })

  it('accepts a single profile selection', () => {
    const parsed = onboardingFormSchema.parse({
      selectedProfileIds: ['testing'],
    })
    expect(parsed.selectedProfileIds).toEqual(['testing'])
  })
})
