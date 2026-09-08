import { describe, expect, it } from 'bun:test'
import { mergeStoredCredentials } from '../../../src/api/routes/provider'

const REAL_URL = 'https://api.real.example/v1'

describe('mergeStoredCredentials', () => {
  it('fills a blank api key from the stored row when type and target match', () => {
    const merged = mergeStoredCredentials(
      { provider: 'openai-compatible', baseUrl: REAL_URL, apiKey: '' },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-stored')
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

  it('does not reuse the stored key when the base URL differs', () => {
    const merged = mergeStoredCredentials(
      {
        provider: 'openai-compatible',
        baseUrl: 'https://attacker.example/v1',
        apiKey: '',
      },
      { type: 'openai-compatible', baseUrl: REAL_URL, apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('')
  })

  it('returns the config unchanged when there is no stored row', () => {
    const config = {
      provider: 'openai-compatible',
      baseUrl: REAL_URL,
      apiKey: '',
    }
    expect(mergeStoredCredentials(config, null)).toBe(config)
  })

  it('fills bedrock credentials when the region matches', () => {
    const merged = mergeStoredCredentials(
      {
        provider: 'bedrock',
        region: 'us-east-1',
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
  })

  it('does not reuse bedrock credentials when the region differs', () => {
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
    expect(merged.accessKeyId).toBe('')
  })
})
