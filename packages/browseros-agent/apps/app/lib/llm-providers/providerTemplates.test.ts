import { describe, expect, it } from 'bun:test'
import { LLMProviderSchema } from '@browseros/shared/schemas/llm'
import { providerFormSchema } from '../../screens/ai-settings/provider-form-schema'
import {
  getDefaultBaseUrlForProviders,
  providerTemplates,
  providerTypeOptions,
} from './providerTemplates'

describe('providerTemplates', () => {
  it('uses ChatGPT as the display name for new ChatGPT providers', () => {
    const template = providerTemplates.find(
      (provider) => provider.id === 'chatgpt-pro',
    )

    expect(template).toMatchObject({
      name: 'ChatGPT',
      defaultModelId: 'gpt-5.5',
      contextWindow: 1050000,
    })
  })
})

it('offers Rapid-MLX with a valid keyless local configuration', () => {
  const template = providerTemplates.find(
    (provider) => provider.id === 'rapid-mlx',
  )
  if (!template) throw new Error('Missing Rapid-MLX template')
  expect(template).toMatchObject({
    name: 'Rapid-MLX',
    defaultBaseUrl: 'http://localhost:8000/v1',
    defaultModelId: 'mlx-community/Qwen3.5-4B-MLX-4bit',
  })
  expect(getDefaultBaseUrlForProviders(template.id)).toBe(
    template.defaultBaseUrl,
  )
  expect(providerTypeOptions).toContainEqual({
    value: template.id,
    label: template.name,
  })
  expect(LLMProviderSchema.parse(template.id)).toBe('rapid-mlx')
  expect(
    providerFormSchema.safeParse({
      type: template.id,
      name: template.name,
      baseUrl: template.defaultBaseUrl,
      modelId: template.defaultModelId,
      supportsImages: template.supportsImages,
      contextWindow: template.contextWindow,
      temperature: 0.2,
    }).success,
  ).toBe(true)
})
