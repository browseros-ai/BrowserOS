import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { detectHostedModelRetirement } from './hosted-model-retirement'

function provider(type: string): LlmProviderConfig {
  return {
    id: `${type}-1`,
    type: type as LlmProviderConfig['type'],
    name: type,
    modelId: 'model',
    supportsImages: true,
    contextWindow: 128000,
    temperature: 0.2,
    createdAt: 1,
    updatedAt: 1,
  }
}

function deps(
  overrides: {
    marked?: boolean
    stored?: LlmProviderConfig[]
    backup?: LlmProviderConfig[]
  } = {},
) {
  const marks: boolean[] = []
  return {
    marks,
    deps: {
      isMarked: async () => overrides.marked ?? false,
      mark: async () => {
        marks.push(true)
      },
      loadStoredProviders: async () => overrides.stored ?? [],
      loadBackupProviders: async () => overrides.backup ?? [],
    },
  }
}

describe('detectHostedModelRetirement', () => {
  it('records a profile that still has the hosted provider in extension storage', async () => {
    const { marks, deps: d } = deps({ stored: [provider('browseros')] })

    expect(await detectHostedModelRetirement(d)).toBe(true)
    expect(marks).toEqual([true])
  })

  it('records a profile whose only evidence is the pref backup', async () => {
    // The reinstall case: extension storage was cleared and the per-profile
    // pref outlived it.
    const { marks, deps: d } = deps({ backup: [provider('browseros')] })

    expect(await detectHostedModelRetirement(d)).toBe(true)
    expect(marks).toEqual([true])
  })

  it('leaves a profile that only ever used its own providers alone', async () => {
    const { marks, deps: d } = deps({
      stored: [provider('openai')],
      backup: [provider('anthropic')],
    })

    expect(await detectHostedModelRetirement(d)).toBe(false)
    expect(marks).toEqual([])
  })

  it('leaves a fresh profile alone', async () => {
    const { marks, deps: d } = deps()

    expect(await detectHostedModelRetirement(d)).toBe(false)
    expect(marks).toEqual([])
  })

  it('does not re-read the evidence once the answer is recorded', async () => {
    // Written once on purpose: the sources are legacy and will be cleaned up,
    // and the answer must survive that.
    let reads = 0
    const result = await detectHostedModelRetirement({
      isMarked: async () => true,
      mark: async () => {
        throw new Error('should not re-mark')
      },
      loadStoredProviders: async () => {
        reads += 1
        return []
      },
      loadBackupProviders: async () => {
        reads += 1
        return []
      },
    })

    expect(result).toBe(true)
    expect(reads).toBe(0)
  })
})
