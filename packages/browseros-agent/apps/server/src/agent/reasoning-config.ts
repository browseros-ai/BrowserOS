import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import type { JSONValue } from 'ai'
import type { ResolvedAgentConfig } from './types'

/** Mirrors the AI SDK's (non-exported) ProviderOptions shape. */
type ProviderOptions = Record<string, Record<string, JSONValue>>

/** AI SDK top-level reasoning setting values. Note there is no 'max'. */
type ReasoningLevel =
  | 'provider-default'
  | 'none'
  | 'minimal'
  | 'low'
  | 'medium'
  | 'high'
  | 'xhigh'

/** Google `thinkingLevel` accepts a narrower set than the reasoning setting. */
type ThinkingLevel = 'minimal' | 'low' | 'medium' | 'high'

/**
 * A partial `ToolLoopAgent` settings object: the top-level `reasoning` call
 * setting (used by providers that translate it, e.g. Anthropic) and/or
 * `providerOptions` (per-provider namespaces).
 */
export interface AgentReasoningConfig {
  reasoning?: ReasoningLevel
  providerOptions?: ProviderOptions
}

const REASONING_LEVELS = new Set<ReasoningLevel>([
  'provider-default',
  'none',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
])

/** Maps a stored effort (which may include 'max') to the SDK reasoning level. */
function toReasoningLevel(effort: string | undefined): ReasoningLevel {
  if (!effort) return 'medium'
  if (effort === 'max') return 'xhigh'
  return REASONING_LEVELS.has(effort as ReasoningLevel)
    ? (effort as ReasoningLevel)
    : 'medium'
}

/** Maps a stored effort to Google's four-value thinking level. */
function toThinkingLevel(effort: string | undefined): ThinkingLevel {
  switch (effort) {
    case 'none':
    case 'minimal':
      return 'minimal'
    case 'low':
      return 'low'
    case 'high':
    case 'xhigh':
    case 'max':
      return 'high'
    default:
      return 'medium'
  }
}

/** Token budget for providers that take an explicit thinking budget. */
function toThinkingBudget(effort: string | undefined): number {
  switch (effort) {
    case 'none':
      return 0
    case 'minimal':
      return 1024
    case 'low':
      return 4096
    case 'medium':
      return 8192
    case 'high':
      return 24576
    case 'xhigh':
    case 'max':
      return 32000
    default:
      return -1 // dynamic: let the model decide
  }
}

function effectiveProvider(cfg: ResolvedAgentConfig): string {
  if (cfg.provider === LLM_PROVIDERS.BROWSEROS && cfg.upstreamProvider) {
    return cfg.upstreamProvider
  }
  return cfg.provider
}

const GEMINI_2_5 = /(^|\/)gemini-2\.5(?:[.-]|$)/i
const GEMINI_3_PLUS = /(^|\/)gemini-(3|[4-9])/i

/**
 * Builds the per-provider reasoning options attached at the single ToolLoopAgent
 * construction site. Driven by the model's reasoning capability
 * (`cfg.supportsReasoning`) plus provider-specific model gating where sending
 * reasoning to a non-reasoning model would error (Google, Bedrock).
 *
 * OPENAI and CHATGPT_PRO are intentionally two independent cases despite sharing
 * the @ai-sdk/openai Responses adapter: the Codex OAuth backend behind
 * CHATGPT_PRO needs stateless reasoning continuity that plain OPENAI must not
 * inherit.
 */
export function buildAgentReasoningConfig(
  cfg: ResolvedAgentConfig,
): AgentReasoningConfig {
  // The catalog says this model does not reason: never request reasoning.
  if (cfg.supportsReasoning === false) return {}

  const provider = effectiveProvider(cfg)
  const effort = cfg.reasoningEffort
  const summary = cfg.reasoningSummary || 'auto'
  const model = cfg.model.toLowerCase()

  switch (provider) {
    case LLM_PROVIDERS.OPENAI:
      return {
        providerOptions: {
          openai: {
            reasoningSummary: summary,
            ...(effort ? { reasoningEffort: effort } : {}),
          },
        },
      }

    case LLM_PROVIDERS.CHATGPT_PRO:
      return {
        providerOptions: {
          openai: {
            store: false,
            reasoningEffort: effort || 'medium',
            reasoningSummary: summary,
            include: ['reasoning.encrypted_content'],
          },
        },
      }

    case LLM_PROVIDERS.AZURE:
      return {
        providerOptions: {
          azure: {
            reasoningEffort: effort || 'medium',
            reasoningSummary: summary,
            // Only force reasoning when the catalog confirms a reasoning model:
            // detection otherwise keys off the user-named Azure deployment.
            ...(cfg.supportsReasoning === true ? { forceReasoning: true } : {}),
          },
        },
      }

    case LLM_PROVIDERS.GOOGLE: {
      if (GEMINI_3_PLUS.test(model)) {
        return {
          providerOptions: {
            google: {
              thinkingConfig: {
                includeThoughts: true,
                thinkingLevel: toThinkingLevel(effort),
              },
            },
          },
        }
      }
      if (GEMINI_2_5.test(model)) {
        return {
          providerOptions: {
            google: {
              thinkingConfig: {
                includeThoughts: true,
                thinkingBudget: toThinkingBudget(effort),
              },
            },
          },
        }
      }
      // Non-thinking Gemini (2.0, gemma): sending thinkingConfig is a 400.
      return {}
    }

    case LLM_PROVIDERS.ANTHROPIC:
      // The Anthropic provider maps the top-level reasoning setting to adaptive
      // display:'summarized' (or an enabled thinking budget) per model.
      return { reasoning: toReasoningLevel(effort) }

    case LLM_PROVIDERS.BEDROCK:
      // Extended thinking on Bedrock is Anthropic-only.
      if (!model.includes('anthropic')) return {}
      return {
        providerOptions: {
          bedrock: {
            reasoningConfig: {
              type: 'enabled',
              budgetTokens: Math.max(1024, toThinkingBudget(effort)),
            },
          },
        },
      }

    // OpenRouter enables reasoning at model-construction time (factory
    // extraBody). openai-compatible providers auto-surface reasoning_content.
    default:
      return {}
  }
}
