import { describe, expect, it } from 'bun:test'
import { providerTypeOptions } from '../../lib/llm-providers/providerTemplates'
import {
  normalizeProviderFormValues,
  providerFormSchema,
} from './provider-form-schema'

const baseValues = {
  name: 'Provider',
  modelId: 'model',
  supportsImages: false,
  contextWindow: 128000,
  temperature: 0.2,
}

describe('provider setup boundary', () => {
  for (const type of ['claude-code', 'codex', 'acp-custom']) {
    it(`rejects removed ACP provider type ${type}`, () => {
      expect(
        providerFormSchema.safeParse({ ...baseValues, type }).success,
      ).toBe(false)
    })
  }

  it('keeps ordinary provider validation unchanged', () => {
    const values = {
      ...baseValues,
      type: 'openai' as const,
      baseUrl: 'https://api.openai.com/v1',
      apiKey: 'secret',
    }

    expect(providerFormSchema.safeParse(values).success).toBe(true)
    expect(normalizeProviderFormValues(values)).toEqual(values)
  })

  it('does not show ACP agents in provider options', () => {
    const values = providerTypeOptions.map((option) => option.value)
    expect(values).not.toContain('claude-code')
    expect(values).not.toContain('codex')
    expect(values).not.toContain('acp-custom')
  })
})
