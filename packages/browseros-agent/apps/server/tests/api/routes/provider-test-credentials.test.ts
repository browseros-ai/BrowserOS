import { describe, expect, it } from 'bun:test'
import { mergeStoredCredentials } from '../../../src/api/routes/provider'

const REAL_URL = 'https://api.real.example/v1'

describe('mergeStoredCredentials', () => {
  it('fills a blank api key from the stored row', () => {
    const merged = mergeStoredCredentials(
      { provider: 'openai-compatible', baseUrl: REAL_URL, apiKey: '' },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-stored')
    expect(merged.baseUrl).toBe(REAL_URL)
  })

  it('keeps a supplied api key instead of the stored one', () => {
    const merged = mergeStoredCredentials(
      { provider: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-new' },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-new')
  })

  it('does not reuse the stored key when the provider type changed', () => {
    const merged = mergeStoredCredentials(
      { provider: 'azure', baseUrl: REAL_URL, apiKey: '' },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('')
  })

  it('pins a reused key to the stored base URL, ignoring a caller-supplied one', () => {
    const merged = mergeStoredCredentials(
      {
        provider: 'openai-compatible',
        baseUrl: 'https://attacker.example/v1',
        apiKey: '',
      },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-stored')
    expect(merged.baseUrl).toBe(REAL_URL)
  })

  it('keeps the caller base URL when the caller supplies its own key', () => {
    const merged = mergeStoredCredentials(
      {
        provider: 'openai-compatible',
        baseUrl: 'https://api.other.example/v1',
        apiKey: 'sk-new',
      },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-new')
    expect(merged.baseUrl).toBe('https://api.other.example/v1')
  })

  it('returns the config unchanged when there is no stored row', () => {
    const config = {
      provider: 'openai-compatible',
      baseUrl: REAL_URL,
      apiKey: '',
    }
    expect(mergeStoredCredentials(config, null)).toBe(config)
  })

  it('fills bedrock credentials and pins them to the stored region', () => {
    const merged = mergeStoredCredentials(
      {
        provider: 'bedrock',
        region: 'eu-west-1',
        accessKeyId: '',
        secretAccessKey: '',
      },
      {
        type: 'bedrock',
        region: 'us-east-1',
        accessKeyId: 'AKIA-stored',
        secretAccessKey: 'secret-stored',
      },
    )
    expect(merged.accessKeyId).toBe('AKIA-stored')
    expect(merged.secretAccessKey).toBe('secret-stored')
    expect(merged.region).toBe('us-east-1')
  })
})
