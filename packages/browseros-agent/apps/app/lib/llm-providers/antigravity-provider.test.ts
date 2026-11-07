import { describe, expect, it } from 'bun:test'
import { categoryForTemplate } from '@/screens/ai-settings/add-provider.helpers'
import { servesUserLoadedModels } from '@/screens/ai-settings/model-picker.helpers'
import { providerFormSchema } from '@/screens/ai-settings/provider-form-schema'
import {
  getDefaultBaseUrlForProviders,
  getProviderTemplate,
  providerTypeOptions,
} from './providerTemplates'

describe('antigravity provider template', () => {
  it('defaults to the local bridge endpoint', () => {
    const template = getProviderTemplate('antigravity')
    expect(template?.defaultBaseUrl).toBe('http://127.0.0.1:8765/v1')
    expect(template?.defaultModelId).toBe('gemini-3.8-flash')
    expect(getDefaultBaseUrlForProviders('antigravity')).toBe(
      'http://127.0.0.1:8765/v1',
    )
  })

  it('is offered in the provider type dropdown', () => {
    expect(providerTypeOptions.map((o) => o.value)).toContain('antigravity')
  })

  it('is grouped as a local provider serving user-chosen models', () => {
    expect(categoryForTemplate('antigravity')).toBe('local')
    expect(servesUserLoadedModels('antigravity')).toBe(true)
  })

  it('is accepted by the provider form schema', () => {
    const result = providerFormSchema.safeParse({
      type: 'antigravity',
      name: 'Antigravity',
      baseUrl: 'http://127.0.0.1:8765/v1',
      modelId: 'gemini-3.8-flash',
      supportsImages: false,
      contextWindow: 1000000,
      temperature: 1,
    })
    expect(result.success).toBe(true)
  })
})
