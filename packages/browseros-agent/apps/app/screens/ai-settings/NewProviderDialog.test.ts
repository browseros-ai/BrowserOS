import { describe, expect, it } from 'bun:test'
import {
  getDefaultBaseUrlForProviders,
  providerTypeOptions,
} from '../../lib/llm-providers/providerTemplates'
import {
  normalizeProviderFormValues,
  providerFormSchema,
} from './provider-form-schema'

const baseProviderValues = {
  name: 'Local coding provider',
  modelId: 'default',
  supportsImages: false,
  contextWindow: 128000,
  temperature: 0.2,
}

describe('providerFormSchema', () => {
  it('accepts Codex provider configs without API credentials', () => {
    const result = providerFormSchema.safeParse({
      ...baseProviderValues,
      type: 'codex',
    })

    expect(result.success).toBe(true)
  })

  it('accepts Claude Code provider configs without API credentials', () => {
    const result = providerFormSchema.safeParse({
      ...baseProviderValues,
      type: 'claude-code',
    })

    expect(result.success).toBe(true)
  })

  it('still requires a base URL for ordinary API-backed providers', () => {
    const result = providerFormSchema.safeParse({
      ...baseProviderValues,
      type: 'openai',
    })

    expect(result.success).toBe(false)
  })
})

describe('normalizeProviderFormValues', () => {
  const staleCredentialValues = {
    ...baseProviderValues,
    baseUrl: 'https://api.openai.com/v1',
    apiKey: 'secret',
    resourceName: 'resource',
    accessKeyId: 'access-key',
    secretAccessKey: 'secret-access-key',
    region: 'us-east-1',
    sessionToken: 'session-token',
  }

  it('clears stale endpoint and credential fields for Codex configs', () => {
    const values = normalizeProviderFormValues({
      ...staleCredentialValues,
      type: 'codex',
    })

    expect(values.baseUrl).toBe('')
    expect(values.apiKey).toBe('')
    expect(values.resourceName).toBe('')
    expect(values.accessKeyId).toBe('')
    expect(values.secretAccessKey).toBe('')
    expect(values.region).toBe('')
    expect(values.sessionToken).toBe('')
  })

  it('clears stale endpoint and credential fields for Claude Code configs', () => {
    const values = normalizeProviderFormValues({
      ...staleCredentialValues,
      type: 'claude-code',
    })

    expect(values.baseUrl).toBe('')
    expect(values.apiKey).toBe('')
    expect(values.resourceName).toBe('')
    expect(values.accessKeyId).toBe('')
    expect(values.secretAccessKey).toBe('')
    expect(values.region).toBe('')
    expect(values.sessionToken).toBe('')
  })

  it('leaves ordinary API-backed provider fields intact', () => {
    const values = normalizeProviderFormValues({
      ...staleCredentialValues,
      type: 'openai',
    })

    expect(values.baseUrl).toBe('https://api.openai.com/v1')
    expect(values.apiKey).toBe('secret')
  })
})

describe('provider type options', () => {
  it('includes Codex and Claude Code with empty base URL defaults', () => {
    expect(providerTypeOptions).toContainEqual({
      value: 'codex',
      label: 'Codex',
    })
    expect(providerTypeOptions).toContainEqual({
      value: 'claude-code',
      label: 'Claude Code',
    })
    expect(getDefaultBaseUrlForProviders('codex')).toBe('')
    expect(getDefaultBaseUrlForProviders('claude-code')).toBe('')
  })
})
