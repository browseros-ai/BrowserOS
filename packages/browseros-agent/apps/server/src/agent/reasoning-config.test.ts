import { describe, expect, test } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { buildAgentReasoningConfig } from './reasoning-config'
import type { ResolvedAgentConfig } from './types'

function cfg(
  overrides: Partial<ResolvedAgentConfig> = {},
): ResolvedAgentConfig {
  return {
    conversationId: 'c1',
    provider: LLM_PROVIDERS.OPENAI,
    model: 'gpt-5',
    ...overrides,
  }
}

describe('buildAgentReasoningConfig', () => {
  test('OPENAI requests a summary without store/include', () => {
    const out = buildAgentReasoningConfig(
      cfg({ provider: LLM_PROVIDERS.OPENAI }),
    )
    expect(out).toEqual({
      providerOptions: { openai: { reasoningSummary: 'auto' } },
    })
  })

  test('OPENAI forwards a configured effort', () => {
    const out = buildAgentReasoningConfig(
      cfg({ provider: LLM_PROVIDERS.OPENAI, reasoningEffort: 'high' }),
    )
    expect(out.providerOptions?.openai).toEqual({
      reasoningSummary: 'auto',
      reasoningEffort: 'high',
    })
  })

  test('CHATGPT_PRO keeps its own decoupled block with store:false + encrypted include', () => {
    const out = buildAgentReasoningConfig(
      cfg({ provider: LLM_PROVIDERS.CHATGPT_PRO }),
    )
    expect(out.providerOptions?.openai).toEqual({
      store: false,
      reasoningEffort: 'medium',
      reasoningSummary: 'auto',
      include: ['reasoning.encrypted_content'],
    })
  })

  test('AZURE uses the azure namespace and forces reasoning only when confirmed', () => {
    const confirmed = buildAgentReasoningConfig(
      cfg({
        provider: LLM_PROVIDERS.AZURE,
        supportsReasoning: true,
        model: 'o3',
      }),
    )
    expect(confirmed.providerOptions?.azure).toEqual({
      reasoningEffort: 'medium',
      reasoningSummary: 'auto',
      forceReasoning: true,
    })

    const unknown = buildAgentReasoningConfig(
      cfg({ provider: LLM_PROVIDERS.AZURE, model: 'my-deployment' }),
    )
    expect(unknown.providerOptions?.azure).toEqual({
      reasoningEffort: 'medium',
      reasoningSummary: 'auto',
    })
  })

  test('GOOGLE gemini-2.5 sends a thinking budget, gemini-3+ sends a level', () => {
    const g25 = buildAgentReasoningConfig(
      cfg({
        provider: LLM_PROVIDERS.GOOGLE,
        model: 'gemini-2.5-pro',
        reasoningEffort: 'high',
      }),
    )
    expect(g25.providerOptions?.google).toEqual({
      thinkingConfig: { includeThoughts: true, thinkingBudget: 24576 },
    })

    const g3 = buildAgentReasoningConfig(
      cfg({
        provider: LLM_PROVIDERS.GOOGLE,
        model: 'gemini-3.5-flash',
        reasoningEffort: 'low',
      }),
    )
    expect(g3.providerOptions?.google).toEqual({
      thinkingConfig: { includeThoughts: true, thinkingLevel: 'low' },
    })
  })

  test('GOOGLE non-thinking model gets no reasoning options', () => {
    expect(
      buildAgentReasoningConfig(
        cfg({ provider: LLM_PROVIDERS.GOOGLE, model: 'gemini-2.0-flash' }),
      ),
    ).toEqual({})
  })

  test('ANTHROPIC uses the top-level reasoning setting with max mapped to xhigh', () => {
    expect(
      buildAgentReasoningConfig(
        cfg({ provider: LLM_PROVIDERS.ANTHROPIC, model: 'claude-opus-4-8' }),
      ),
    ).toEqual({ reasoning: 'medium' })

    expect(
      buildAgentReasoningConfig(
        cfg({ provider: LLM_PROVIDERS.ANTHROPIC, reasoningEffort: 'max' }),
      ),
    ).toEqual({ reasoning: 'xhigh' })
  })

  test('BEDROCK enables thinking only for anthropic models', () => {
    const claude = buildAgentReasoningConfig(
      cfg({
        provider: LLM_PROVIDERS.BEDROCK,
        model: 'us.anthropic.claude-sonnet-4-5-20250101-v1:0',
        reasoningEffort: 'medium',
      }),
    )
    expect(claude.providerOptions?.bedrock).toEqual({
      reasoningConfig: { type: 'enabled', budgetTokens: 8192 },
    })

    expect(
      buildAgentReasoningConfig(
        cfg({ provider: LLM_PROVIDERS.BEDROCK, model: 'amazon.nova-pro-v1:0' }),
      ),
    ).toEqual({})
  })

  test('OPENROUTER and openai-compatible get no agent-level reasoning options', () => {
    expect(
      buildAgentReasoningConfig(cfg({ provider: LLM_PROVIDERS.OPENROUTER })),
    ).toEqual({})
    expect(
      buildAgentReasoningConfig(
        cfg({ provider: LLM_PROVIDERS.OPENAI_COMPATIBLE }),
      ),
    ).toEqual({})
  })

  test('BROWSEROS resolves via upstreamProvider', () => {
    const out = buildAgentReasoningConfig(
      cfg({
        provider: LLM_PROVIDERS.BROWSEROS,
        upstreamProvider: LLM_PROVIDERS.ANTHROPIC,
        reasoningEffort: 'high',
      }),
    )
    expect(out).toEqual({ reasoning: 'high' })
  })

  test('a non-reasoning model is skipped for every provider', () => {
    for (const provider of [
      LLM_PROVIDERS.OPENAI,
      LLM_PROVIDERS.AZURE,
      LLM_PROVIDERS.GOOGLE,
      LLM_PROVIDERS.ANTHROPIC,
      LLM_PROVIDERS.BEDROCK,
    ]) {
      expect(
        buildAgentReasoningConfig(
          cfg({ provider, supportsReasoning: false, model: 'gpt-5' }),
        ),
      ).toEqual({})
    }
  })
})
