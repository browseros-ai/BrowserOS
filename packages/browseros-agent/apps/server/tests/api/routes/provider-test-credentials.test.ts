import { describe, expect, it } from 'bun:test'
import { mergeStoredCredentials } from '../../../src/api/routes/provider'

describe('mergeStoredCredentials', () => {
  it('fills a blank api key from the stored row when the type matches', () => {
    const merged = mergeStoredCredentials(
      { provider: 'openai-compatible', apiKey: '' },
      { type: 'openai-compatible', apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-stored')
  })

  it('keeps a supplied api key instead of the stored one', () => {
    const merged = mergeStoredCredentials(
      { provider: 'openai-compatible', apiKey: 'sk-new' },
      { type: 'openai-compatible', apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('sk-new')
  })

  it('does not reuse the stored key when the provider type changed', () => {
    const merged = mergeStoredCredentials(
      { provider: 'azure', apiKey: '' },
      { type: 'openai-compatible', apiKey: 'sk-stored' },
    )
    expect(merged.apiKey).toBe('')
  })

  it('returns the config unchanged when there is no stored row', () => {
    const config = { provider: 'openai-compatible', apiKey: '' }
    expect(mergeStoredCredentials(config, null)).toBe(config)
  })

  it('fills bedrock credentials from the stored row', () => {
    const merged = mergeStoredCredentials(
      { provider: 'bedrock', accessKeyId: '', secretAccessKey: '' },
      {
        type: 'bedrock',
        accessKeyId: 'AKIA-stored',
        secretAccessKey: 'secret-stored',
      },
    )
    expect(merged.accessKeyId).toBe('AKIA-stored')
    expect(merged.secretAccessKey).toBe('secret-stored')
  })
})
