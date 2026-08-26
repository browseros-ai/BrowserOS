/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import { type ModelMessage, pruneMessages } from 'ai'
import { logger } from '../lib/logger'
import { stripBinaryContent } from './compaction/content'
import {
  estimateTokens,
  estimateTotalTokens,
  getCurrentTokenCount,
  type StepWithUsage,
} from './compaction/tokens'

export type { StepWithUsage }
export { estimateTokens, estimateTotalTokens, getCurrentTokenCount }

export interface CompactionConfig {
  contextWindow: number
}

export interface CompactionBudget {
  /** Context window actually used, after rejecting nonsense values. */
  contextWindow: number
  /** Compaction runs once the estimated prompt exceeds this. */
  threshold: number
  /** System prompt and tool schemas, which `prepareStep` cannot see. */
  overhead: number
}

/**
 * Derives the single trigger threshold from the context window.
 *
 * Reserve leaves room for the model's own response. It is capped at half the
 * window so small models are not left with a negative budget, and the overhead
 * allowance is capped alongside it so overhead alone can never exceed the
 * threshold and pin compaction on permanently.
 *
 * `contextWindowSize` arrives from the client as a bare optional number, so a
 * zero, a negative, or a NaN can reach here and would otherwise produce a
 * threshold that every request exceeds.
 */
export function computeBudget(contextWindow: number): CompactionBudget {
  const window =
    Number.isFinite(contextWindow) && contextWindow > 0
      ? Math.floor(contextWindow)
      : AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW
  const reserve = Math.min(
    AGENT_LIMITS.COMPACTION_RESERVE_TOKENS,
    Math.floor(window * 0.5),
  )
  const overhead = Math.min(
    AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD,
    Math.floor(window * 0.4),
  )
  return { contextWindow: window, threshold: window - reserve, overhead }
}

/**
 * Builds the `prepareStep` callback that keeps a run inside its context window.
 *
 * The policy is deliberately small, following the AI SDK's compaction guide:
 * one trigger threshold, one compaction path, one deterministic fallback.
 *
 * The system prompt is never at risk here. It reaches the model as the agent's
 * `instructions`, which `prepareStep` receives separately from `messages` and
 * which this callback never returns, so it carries through every compaction
 * untouched. It is only relevant to the token math, where it is accounted for
 * as overhead.
 */
export function createCompactionPrepareStep(
  userConfig?: Partial<CompactionConfig>,
) {
  const { contextWindow, threshold, overhead } = computeBudget(
    userConfig?.contextWindow ?? AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW,
  )
  const keepRecent = AGENT_LIMITS.COMPACTION_PRUNE_KEEP_RECENT_MESSAGES

  logger.info('Compaction configured', {
    contextWindow,
    threshold,
    overhead,
    keepRecentMessages: keepRecent,
  })

  return async ({
    messages,
    steps,
  }: {
    messages: ModelMessage[]
    steps: ReadonlyArray<StepWithUsage>
  }): Promise<{ messages: ModelMessage[] }> => {
    const currentTokens = getCurrentTokenCount(steps, messages, overhead)
    if (currentTokens <= threshold) {
      return { messages }
    }

    // One path: drop binary payloads, then let the SDK prune reasoning and the
    // older tool call/result/approval chunks. Pruning through the SDK is what
    // keeps tool calls paired with their results.
    const compacted = pruneMessages({
      messages: stripBinaryContent(messages),
      reasoning: 'all',
      toolCalls: `before-last-${keepRecent}-messages`,
      emptyMessages: 'remove',
    })

    const compactedTokens = estimateTotalTokens(compacted, overhead)
    if (compactedTokens <= threshold) {
      logger.info('Compacted context', {
        currentTokens,
        compactedTokens,
        threshold,
        before: messages.length,
        after: compacted.length,
      })
      return { messages: compacted }
    }

    // One fallback: drop every remaining tool exchange. Deterministic, and the
    // last thing available before the request would overflow.
    const floor = pruneMessages({
      messages: compacted,
      reasoning: 'all',
      toolCalls: 'all',
      emptyMessages: 'remove',
    })

    logger.warn('Compaction fell back to clearing all tool calls', {
      currentTokens,
      compactedTokens,
      floorTokens: estimateTotalTokens(floor, overhead),
      threshold,
      before: messages.length,
      after: floor.length,
    })

    return { messages: floor }
  }
}
