/** Provider fixtures must be accepted by the installed SDK adapters, not merely look like SSE. */
import { expect, test } from 'bun:test'
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import { startFakeProvider } from './fake-provider'

const serverRequire = createRequire(
  resolve(import.meta.dir, '../../apps/server/package.json'),
)
const { streamText } = serverRequire(
  'ai',
) as typeof import('../../apps/server/node_modules/ai')
const { createOpenAI } = serverRequire(
  '@ai-sdk/openai',
) as typeof import('../../apps/server/node_modules/@ai-sdk/openai')
const { createOpenAICompatible } = serverRequire(
  '@ai-sdk/openai-compatible',
) as typeof import('../../apps/server/node_modules/@ai-sdk/openai-compatible')
const { createAnthropic } = serverRequire(
  '@ai-sdk/anthropic',
) as typeof import('../../apps/server/node_modules/@ai-sdk/anthropic')

test('tests that real SDK adapters decode text, reasoning, tool calls and errors in all three dialects', async () => {
  const fake = startFakeProvider({ delayMs: 0 })
  try {
    const adapters = [
      createOpenAICompatible({
        name: 'compat',
        baseURL: `${fake.url}/v1`,
        apiKey: 'fake',
      }),
      createOpenAI({ baseURL: `${fake.url}/v1`, apiKey: 'fake' }).responses,
      createAnthropic({ baseURL: `${fake.url}/v1`, apiKey: 'fake' }),
    ]
    for (const adapter of adapters) {
      for (const scenario of [
        'plain-text',
        'reasoning-text',
        'one-tool',
        'parallel-tools',
        'mid-stream-error',
      ] as const) {
        const result = streamText({
          model: adapter(scenario),
          prompt: 'Fixture',
          maxRetries: 0,
          maxOutputTokens: 1000,
          onError: () => {}, // Error chunks below are the assertion surface; skip the SDK's default console logger.
        })
        const parts = []
        for await (const part of result.fullStream) parts.push(part)
        const errors = parts.filter((part) => part.type === 'error')
        if (scenario === 'mid-stream-error')
          expect(errors.length).toBeGreaterThan(0)
        else if (scenario.includes('tool')) {
          // No tools supplied here: translation still exposes named calls, followed by SDK NoSuchTool errors.
          expect(parts.filter((part) => part.type === 'tool-call').length).toBe(
            scenario === 'parallel-tools' ? 2 : 1,
          )
        } else {
          expect(errors).toEqual([])
          expect(
            parts
              .filter((part) => part.type === 'text-delta')
              .map((part) => part.text)
              .join(''),
          ).toBe('Hello fixture.')
          if (scenario === 'reasoning-text')
            expect(
              parts
                .filter((part) => part.type === 'reasoning-delta')
                .map((part) => part.text)
                .join(''),
            ).toBe('Thinking.')
        }
      }
    }
  } finally {
    await fake.stop()
  }
})
