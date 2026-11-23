import { describe, expect, it } from 'bun:test'
import { providerTypeOptions } from '../../lib/llm-providers/providerTemplates'
import {
  normalizeProviderFormValues,
  providerFormSchema,
  sameProviderAddress,
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

  it('requires the API key for a new Azure provider', () => {
    const result = providerFormSchema.safeParse({
      ...baseValues,
      type: 'azure' as const,
      baseUrl: 'https://example.openai.azure.com',
      apiKey: '',
    })
    expect(result.success).toBe(false)
  })

  it('lets an Azure provider save blank when a key is already stored', () => {
    const result = providerFormSchema.safeParse({
      ...baseValues,
      type: 'azure' as const,
      baseUrl: 'https://example.openai.azure.com',
      apiKey: '',
      hasApiKey: true,
      originalType: 'azure' as const,
      originalBaseUrl: 'https://example.openai.azure.com',
    })
    expect(result.success).toBe(true)
  })

  it('re-requires the credential when an edited provider switches type', () => {
    const result = providerFormSchema.safeParse({
      ...baseValues,
      type: 'azure' as const,
      baseUrl: 'https://example.openai.azure.com',
      apiKey: '',
      hasApiKey: true,
      originalType: 'openai-compatible' as const,
    })
    expect(result.success).toBe(false)
  })

  it('requires AWS credentials for a new Bedrock provider', () => {
    const result = providerFormSchema.safeParse({
      ...baseValues,
      type: 'bedrock' as const,
      region: 'us-east-1',
      accessKeyId: '',
      secretAccessKey: '',
    })
    expect(result.success).toBe(false)
  })

  it('lets a Bedrock provider save blank when credentials are already stored', () => {
    const result = providerFormSchema.safeParse({
      ...baseValues,
      type: 'bedrock' as const,
      region: 'us-east-1',
      accessKeyId: '',
      secretAccessKey: '',
      hasAccessKeyId: true,
      hasSecretAccessKey: true,
      originalType: 'bedrock' as const,
      originalRegion: 'us-east-1',
    })
    expect(result.success).toBe(true)
  })
})

function issuePaths(input: unknown): string[] {
  const result = providerFormSchema.safeParse(input)
  return result.success
    ? []
    : result.error.issues.map((issue) => issue.path.join('.'))
}

describe('editing a provider whose key is stored', () => {
  // The stored row as the dialog reads it back: the key itself never arrives,
  // only the flag, and the original* fields carry the row's raw address.
  const storedOpenAi = {
    ...baseValues,
    type: 'openai' as const,
    baseUrl: 'https://api.openai.com/v1',
    apiKey: '',
    hasApiKey: true,
    originalType: 'openai' as const,
    originalBaseUrl: 'https://api.openai.com/v1',
  }
  const storedAzure = {
    ...baseValues,
    type: 'azure' as const,
    resourceName: 'my-resource',
    baseUrl: '',
    apiKey: '',
    hasApiKey: true,
    originalType: 'azure' as const,
    originalResourceName: 'my-resource',
  }
  const storedBedrock = {
    ...baseValues,
    type: 'bedrock' as const,
    region: 'us-east-1',
    accessKeyId: '',
    secretAccessKey: '',
    hasAccessKeyId: true,
    hasSecretAccessKey: true,
    originalType: 'bedrock' as const,
    originalRegion: 'us-east-1',
  }

  it('keeps the stored key when the address is unchanged', () => {
    expect(issuePaths(storedOpenAi)).toEqual([])
    expect(issuePaths(storedAzure)).toEqual([])
    expect(issuePaths(storedBedrock)).toEqual([])
  })

  it('asks for the key again when the base URL changes', () => {
    expect(
      issuePaths({ ...storedOpenAi, baseUrl: 'https://proxy.example.com/v1' }),
    ).toEqual(['apiKey'])
    expect(
      issuePaths({ ...storedAzure, baseUrl: 'https://other.example.com' }),
    ).toEqual(['apiKey'])
  })

  it('accepts a newly typed key at a changed base URL', () => {
    expect(
      issuePaths({
        ...storedOpenAi,
        baseUrl: 'https://proxy.example.com/v1',
        apiKey: 'sk-new',
      }),
    ).toEqual([])
  })

  it('asks for the key again when the type changes', () => {
    expect(
      issuePaths({
        ...storedOpenAi,
        type: 'anthropic' as const,
        baseUrl: 'https://api.openai.com/v1',
      }),
    ).toEqual(['apiKey'])
  })

  it('asks for the Azure key again when the resource name changes', () => {
    expect(
      issuePaths({ ...storedAzure, resourceName: 'other-resource' }),
    ).toEqual(['apiKey'])
  })

  it('asks for both AWS keys again when the Bedrock region changes', () => {
    expect(issuePaths({ ...storedBedrock, region: 'eu-west-1' })).toEqual([
      'accessKeyId',
      'secretAccessKey',
    ])
  })

  it('treats a default the form filled in for an absent base URL as a change', () => {
    // A row stored without a base URL opens with the type's default filled in;
    // the server compares the save against the absent value, so it clears the key.
    expect(issuePaths({ ...storedOpenAi, originalBaseUrl: undefined })).toEqual(
      ['apiKey'],
    )
  })

  it('leaves an optional key optional at a changed address', () => {
    expect(
      issuePaths({
        ...storedOpenAi,
        type: 'ollama' as const,
        originalType: 'ollama' as const,
        baseUrl: 'http://192.168.1.5:11434/v1',
        originalBaseUrl: 'http://localhost:11434/v1',
      }),
    ).toEqual([])
  })

  it('lets an OpenAI-compatible provider move to a keyless gateway', () => {
    expect(
      issuePaths({
        ...storedOpenAi,
        type: 'openai-compatible' as const,
        originalType: 'openai-compatible' as const,
        baseUrl: 'https://keyless-gateway.example/v1',
        originalBaseUrl: 'https://gateway.example/v1',
      }),
    ).toEqual([])
  })
})

describe('sameProviderAddress', () => {
  it('treats a blank value as absent', () => {
    for (const blank of ['', '   ', undefined]) {
      expect(
        sameProviderAddress(
          { type: 'openai', baseUrl: blank },
          { type: 'openai' },
        ),
      ).toBe(true)
      expect(
        sameProviderAddress(
          { type: 'azure', baseUrl: blank, resourceName: 'r' },
          { type: 'azure', baseUrl: '', resourceName: 'r' },
        ),
      ).toBe(true)
    }
  })

  it('compares any other value exactly as written', () => {
    expect(
      sameProviderAddress(
        { type: 'openai', baseUrl: 'https://api.openai.com/v1/' },
        { type: 'openai', baseUrl: 'https://api.openai.com/v1' },
      ),
    ).toBe(false)
    expect(
      sameProviderAddress(
        { type: 'openai', baseUrl: ' https://api.openai.com/v1' },
        { type: 'openai', baseUrl: 'https://api.openai.com/v1' },
      ),
    ).toBe(false)
  })

  it('compares only the fields each type sends its key to', () => {
    expect(
      sameProviderAddress(
        { type: 'bedrock', region: 'us-east-1', baseUrl: 'https://a' },
        { type: 'bedrock', region: 'us-east-1', baseUrl: 'https://b' },
      ),
    ).toBe(true)
    expect(
      sameProviderAddress(
        {
          type: 'openai',
          baseUrl: 'https://a',
          resourceName: 'x',
          region: 'y',
        },
        { type: 'openai', baseUrl: 'https://a' },
      ),
    ).toBe(true)
    expect(
      sameProviderAddress(
        { type: 'chatgpt-pro', baseUrl: 'https://chatgpt.com/backend-api' },
        { type: 'chatgpt-pro' },
      ),
    ).toBe(true)
    expect(
      sameProviderAddress(
        { type: 'azure', baseUrl: 'https://a', resourceName: 'r1' },
        { type: 'azure', baseUrl: 'https://a', resourceName: 'r2' },
      ),
    ).toBe(false)
  })

  it('counts a change of type as a change of address', () => {
    expect(
      sameProviderAddress(
        { type: 'anthropic', baseUrl: 'https://a' },
        { type: 'openai', baseUrl: 'https://a' },
      ),
    ).toBe(false)
    expect(sameProviderAddress({ type: 'openai' }, {})).toBe(false)
  })
})
