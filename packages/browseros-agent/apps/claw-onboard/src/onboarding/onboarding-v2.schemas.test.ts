import { describe, expect, it } from 'bun:test'
import {
  MOCK_BROWSEROS_IMPORT_SOURCES,
  selectableItemsForSource,
} from './onboarding-v2.helpers'
import {
  onboardingFormDefaults,
  onboardingFormResolver,
  onboardingFormSchema,
} from './onboarding-v2.schemas'

describe('onboardingFormSchema', () => {
  it('accepts the default values', () => {
    const parsed = onboardingFormSchema.parse(onboardingFormDefaults)
    expect(parsed.selectedSourceId).toBe('chrome-work')
    expect(parsed.selectedItems).toEqual(
      selectableItemsForSource(MOCK_BROWSEROS_IMPORT_SOURCES[0]),
    )
  })

  it('rejects an empty selection with a helpful message', () => {
    const result = onboardingFormSchema.safeParse({ selectedSourceId: '' })
    expect(result.success).toBe(false)
    if (!result.success) {
      expect(result.error.issues[0]?.message).toBe('Pick a profile.')
    }
  })

  it('accepts dynamic Chromium source ids', () => {
    const result = onboardingFormSchema.safeParse({
      selectedSourceId: 'source-42',
      selectedItems: ['history'],
    })
    expect(result.success).toBe(true)
  })

  it('preserves string selected items and drops non-string entries', () => {
    const result = onboardingFormSchema.safeParse({
      selectedSourceId: 'source-42',
      selectedItems: ['history', 42, 'savedWindows', null],
    })

    expect(result.success).toBe(true)
    if (result.success) {
      const selectedItems: string[] = result.data.selectedItems
      expect(selectedItems).toEqual(['history', 'savedWindows'])
    }
  })

  it('uses an empty selectedItems array when the field is missing or invalid', () => {
    expect(
      onboardingFormSchema.parse({ selectedSourceId: 'source-42' })
        .selectedItems,
    ).toEqual([])
    expect(
      onboardingFormSchema.parse({
        selectedSourceId: 'source-42',
        selectedItems: 'history',
      }).selectedItems,
    ).toEqual([])
  })

  it('keeps selected items in resolver values for valid submissions', () => {
    const result = onboardingFormResolver(
      { selectedSourceId: 'source-42', selectedItems: ['history'] },
      undefined,
      {} as never,
    )

    expect(result).toEqual({
      values: { selectedSourceId: 'source-42', selectedItems: ['history'] },
      errors: {},
    })
  })
})
