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
 * zero, a negative, a fraction, or a NaN can reach here and would otherwise
 * produce a threshold that every request exceeds. The bound is `>= 1` rather
 * than `> 0` because anything below one floors to zero.
 */
export function computeBudget(contextWindow: number): CompactionBudget {
  const window =
    Number.isFinite(contextWindow) && contextWindow >= 1
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
 * Drops the oldest messages until the transcript fits, keeping the first
 * message and the longest recent suffix that still fits alongside it.
 *
 * This only ever runs after every tool call has already been pruned away, so
 * unlike a general sliding window it cannot separate a tool call from its
 * result: there are no pairs left to break. It exists because pruning has no
 * lever against plain user and assistant text, which is what a transcript is
 * made of once the tool exchanges are gone.
 *
 * Per-message estimates are summed rather than measured across the whole list,
 * which rounds up on every message. Erring high is the safe direction here.
 */
function dropOldestMessages(
  messages: ModelMessage[],
  threshold: number,
  overhead: number,
): ModelMessage[] {
  if (messages.length <= 2) return messages

  const costs = messages.map((message) => estimateTokens([message]))
  let budget = threshold - overhead - costs[0]
  let start = messages.length

  for (let i = messages.length - 1; i >= 1; i--) {
    if (costs[i] > budget) break
    budget -= costs[i]
    start = i
  }

  if (start >= messages.length) {
    return [messages[0], messages[messages.length - 1]]
  }
  return [messages[0], ...messages.slice(start)]
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

    // One fallback, in two moves. First clear every remaining tool exchange.
    const cleared = pruneMessages({
      messages: compacted,
      reasoning: 'all',
      toolCalls: 'all',
      emptyMessages: 'remove',
    })

    const clearedTokens = estimateTotalTokens(cleared, overhead)
    if (clearedTokens <= threshold) {
      logger.warn('Compaction cleared all tool calls', {
        currentTokens,
        compactedTokens,
        clearedTokens,
        threshold,
        before: messages.length,
        after: cleared.length,
      })
      return { messages: cleared }
    }

    // What is left is plain text, which pruning cannot touch, so drop whole
    // messages. Without this the request goes out over the model's limit and
    // the provider rejects it.
    const floor = dropOldestMessages(cleared, threshold, overhead)
    const floorTokens = estimateTotalTokens(floor, overhead)

    logger.warn('Compaction dropped oldest messages', {
      currentTokens,
      compactedTokens,
      clearedTokens,
      floorTokens,
      threshold,
      before: messages.length,
      after: floor.length,
      // A single message larger than the window cannot be shrunk by dropping
      // whole messages. Surfaced so an overflow is diagnosable rather than a
      // bare provider error.
      stillOverBudget: floorTokens > threshold,
    })

    return { messages: floor }
  }
}
