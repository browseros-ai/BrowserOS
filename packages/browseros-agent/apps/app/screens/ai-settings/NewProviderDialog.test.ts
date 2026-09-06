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
  it('normalizes custom headers and keeps conversation placeholders for the server', () => {
    const values = providerFormSchema.parse({
      ...baseValues,
      type: 'openai-compatible',
      baseUrl: 'http://localhost:1234',
      headers: [{ name: 'x-opencode-session', value: '{{conversationId}}' }],
    })
    expect(normalizeProviderFormValues(values).headers).toEqual({
      'x-opencode-session': '{{conversationId}}',
    })
    expect(
      normalizeProviderFormValues({ ...values, headers: [] }).headers,
    ).toEqual({})
  })

  it.each([
    [{ name: '', value: 'a' }],
    [{ name: 'bad name', value: 'a' }],
    [{ name: 'x-test', value: 'a\nb' }],
    [
      { name: 'X-Test', value: 'a' },
      { name: 'x-test', value: 'b' },
    ],
  ])('rejects invalid header rows %j', (...headers) => {
    expect(
      providerFormSchema.safeParse({
        ...baseValues,
        type: 'openai-compatible',
        baseUrl: 'http://localhost:1234',
        headers,
      }).success,
    ).toBe(false)
  })

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
