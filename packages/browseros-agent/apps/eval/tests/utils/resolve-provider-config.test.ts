import { describe, expect, it } from 'bun:test'
import { resolveProviderConfig } from '../../src/utils/resolve-provider-config'

describe('resolveProviderConfig', () => {
  it('resolves Bedrock region from environment variables', async () => {
    const previous = {
      AWS_REGION: process.env.AWS_REGION,
      AWS_ACCESS_KEY_ID: process.env.AWS_ACCESS_KEY_ID,
      AWS_SECRET_ACCESS_KEY: process.env.AWS_SECRET_ACCESS_KEY,
    }
    process.env.AWS_REGION = 'us-west-2'
    process.env.AWS_ACCESS_KEY_ID = 'test-access-key'
    process.env.AWS_SECRET_ACCESS_KEY = 'test-secret-key'

    try {
      const resolved = await resolveProviderConfig({
        provider: 'bedrock',
        model: 'global.anthropic.claude-opus-4-6-v1',
        region: 'AWS_REGION',
        accessKeyId: 'AWS_ACCESS_KEY_ID',
        secretAccessKey: 'AWS_SECRET_ACCESS_KEY',
      })

      expect(resolved).toMatchObject({
        provider: 'bedrock',
        model: 'global.anthropic.claude-opus-4-6-v1',
        region: process.env.AWS_REGION,
        accessKeyId: process.env.AWS_ACCESS_KEY_ID,
        secretAccessKey: process.env.AWS_SECRET_ACCESS_KEY,
      })
    } finally {
      for (const [key, value] of Object.entries(previous)) {
        if (value === undefined) delete process.env[key]
        else process.env[key] = value
      }
    }
  })
})
