import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import type {
  AssistantContent,
  ModelMessage,
  ToolContent,
  UserContent,
} from 'ai'
import { logger } from '../../lib/logger'
import {
  estimateToolResultOutput,
  stripToolResultOutput,
  toolResultOutputToText,
} from './content'

export interface ComputedConfig {
  contextWindow: number
  reserveTokens: number
  triggerRatio: number
  triggerThreshold: number
  keepRecentTokens: number
  minSummarizableTokens: number
  maxSummarizationInput: number
  summarizerMaxOutputTokens: number
  summarizationTimeoutMs: number
  fixedOverhead: number
  safetyMultiplier: number
  imageTokenEstimate: number
  toolOutputMaxChars: number
}

export type CompactionState = {
  existingSummary: string | null
  compactionCount: number
}

export interface StepWithUsage {
  usage?: {
    inputTokens?: number | undefined
    outputTokens?: number | undefined
  }
}

export interface SplitPointResult {
  splitIndex: number
  turnStartIndex: number
  isSplitTurn: boolean
}

export interface OutputReductionOptions {
  maxChars: number
  clearThreshold?: number
  keepRecentCount?: number
}

function safeJsonStringify(value: unknown): string {
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

export function computeConfig(contextWindow: number): ComputedConfig {
  const reserveTokens =
    contextWindow <= AGENT_LIMITS.COMPACTION_SMALL_CONTEXT_WINDOW
      ? Math.floor(contextWindow * 0.5)
      : AGENT_LIMITS.COMPACTION_RESERVE_TOKENS
  const triggerThreshold = Math.max(0, contextWindow - reserveTokens)
  const triggerRatio = contextWindow > 0 ? triggerThreshold / contextWindow : 0

  const baseMinSummarizableTokens =
    contextWindow <= AGENT_LIMITS.COMPACTION_SMALL_CONTEXT_WINDOW
      ? AGENT_LIMITS.COMPACTION_MIN_SUMMARIZABLE_INPUT_SMALL
      : AGENT_LIMITS.COMPACTION_MIN_SUMMARIZABLE_INPUT

  const keepRecentTokens = Math.max(
    0,
    Math.min(
      AGENT_LIMITS.COMPACTION_MAX_KEEP_RECENT,
      Math.floor(
        triggerThreshold * AGENT_LIMITS.COMPACTION_KEEP_RECENT_FRACTION,
      ),
    ),
  )

  const availableToSummarize = Math.max(0, triggerThreshold - keepRecentTokens)

  const minSummarizableTokens = Math.max(
    AGENT_LIMITS.COMPACTION_MIN_TOKEN_FLOOR,
    Math.min(baseMinSummarizableTokens, availableToSummarize),
  )

  const maxSummarizationInput = Math.min(
    AGENT_LIMITS.COMPACTION_MAX_SUMMARIZATION_INPUT,
    Math.max(minSummarizableTokens, availableToSummarize),
  )

  const summarizerMaxOutputTokens = Math.max(
    AGENT_LIMITS.COMPACTION_MIN_TOKEN_FLOOR,
    Math.floor(reserveTokens * AGENT_LIMITS.COMPACTION_SUMMARIZER_OUTPUT_RATIO),
  )

  // Cap overhead so it never exceeds 40% of context — prevents the doom loop
  // where overhead alone triggers compaction on small-context models.
  const fixedOverhead = Math.min(
    AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD,
    Math.floor(contextWindow * 0.4),
  )

  return {
    contextWindow,
    reserveTokens,
    triggerRatio,
    triggerThreshold,
    keepRecentTokens,
    minSummarizableTokens,
    maxSummarizationInput,
    summarizerMaxOutputTokens,
    summarizationTimeoutMs: AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS,
    fixedOverhead,
    safetyMultiplier: AGENT_LIMITS.COMPACTION_SAFETY_MULTIPLIER,
    imageTokenEstimate: AGENT_LIMITS.COMPACTION_IMAGE_TOKEN_ESTIMATE,
    toolOutputMaxChars: AGENT_LIMITS.COMPACTION_TOOL_OUTPUT_MAX_CHARS,
  }
}

function estimateUserContent(content: UserContent): {
  chars: number
  images: number
} {
  if (typeof content === 'string') {
    return { chars: content.length, images: 0 }
  }

  let chars = 0
  let images = 0

  for (const part of content) {
    if (part.type === 'text') {
      chars += part.text.length
    } else if (part.type === 'image' || part.type === 'file') {
      images++
    }
  }

  return { chars, images }
}

function estimateAssistantContent(content: AssistantContent): {
  chars: number
  images: number
} {
  if (typeof content === 'string') {
    return { chars: content.length, images: 0 }
  }

  let chars = 0
  let images = 0

  for (const part of content) {
    switch (part.type) {
      case 'text':
      case 'reasoning':
        chars += part.text.length
        break
      case 'tool-call':
        chars += safeJsonStringify(part.input).length
        break
      case 'tool-result': {
        const estimate = estimateToolResultOutput(part.output)
        chars += estimate.chars
        images += estimate.images
        break
      }
      case 'tool-approval-request':
        chars += part.approvalId.length + part.toolCallId.length
        break
      case 'file':
        images++
        break
    }
  }

  return { chars, images }
}

function estimateToolContent(content: ToolContent): {
  chars: number
  images: number
} {
  let chars = 0
  let images = 0

  for (const part of content) {
    if (part.type === 'tool-result') {
      const estimate = estimateToolResultOutput(part.output)
      chars += estimate.chars
      images += estimate.images
    } else {
      chars += part.approvalId.length
      if (part.reason) {
        chars += part.reason.length
      }
    }
  }

  return { chars, images }
}

export function estimateTokens(
  messages: ModelMessage[],
  imageTokenEstimate: number = AGENT_LIMITS.COMPACTION_IMAGE_TOKEN_ESTIMATE,
): number {
  let chars = 0
  let imageCount = 0

  for (const msg of messages) {
    let estimate = { chars: 0, images: 0 }

    switch (msg.role) {
      case 'system':
        estimate = { chars: msg.content.length, images: 0 }
        break
      case 'user':
        estimate = estimateUserContent(msg.content)
        break
      case 'assistant':
        estimate = estimateAssistantContent(msg.content)
        break
      case 'tool':
        estimate = estimateToolContent(msg.content)
        break
    }

    chars += estimate.chars
    imageCount += estimate.images
  }

  return Math.ceil(chars / 3) + imageCount * imageTokenEstimate
}

export function getCurrentTokenCount(
  steps: ReadonlyArray<StepWithUsage>,
  messages: ModelMessage[],
  config: ComputedConfig,
): number {
  if (steps.length > 0) {
    const lastStep = steps[steps.length - 1]
    if (lastStep.usage?.inputTokens != null && lastStep.usage.inputTokens > 0) {
      const base = lastStep.usage.inputTokens
      const outputTokens = lastStep.usage.outputTokens ?? 0
      const lastAssistantIndex = messages.findLastIndex(
        (message) => message.role === 'assistant',
      )

      if (lastAssistantIndex === -1) {
        return base + outputTokens
      }

      let trailingTokens = 0
      for (let i = messages.length - 1; i > lastAssistantIndex; i--) {
        if (messages[i].role === 'system') continue
        trailingTokens += estimateTokens(
          [messages[i]],
          config.imageTokenEstimate,
        )
      }

      return base + outputTokens + trailingTokens
    }
  }

  const estimated = estimateTokens(messages, config.imageTokenEstimate)
  return Math.ceil(estimated * config.safetyMultiplier) + config.fixedOverhead
}

export function estimateTokensForThreshold(
  messages: ModelMessage[],
  config: ComputedConfig,
): number {
  return (
    Math.ceil(
      estimateTokens(messages, config.imageTokenEstimate) *
        config.safetyMultiplier,
    ) + config.fixedOverhead
  )
}

export function findSafeSplitPoint(
  messages: ModelMessage[],
  keepRecentTokens: number,
  imageTokenEstimate: number = AGENT_LIMITS.COMPACTION_IMAGE_TOKEN_ESTIMATE,
): SplitPointResult {
  const noSplit: SplitPointResult = {
    splitIndex: -1,
    turnStartIndex: -1,
    isSplitTurn: false,
  }

  if (messages.length <= 2) return noSplit

  let accumulated = 0
  let candidateIndex = -1

  for (let i = messages.length - 1; i >= 0; i--) {
    accumulated += estimateTokens([messages[i]], imageTokenEstimate)

    if (accumulated >= keepRecentTokens) {
      candidateIndex = i
      break
    }
  }

  if (candidateIndex === -1) return noSplit

  while (candidateIndex > 0 && messages[candidateIndex].role === 'tool') {
    candidateIndex--
  }

  if (candidateIndex <= 0) return noSplit

  if (messages[candidateIndex].role === 'user') {
    return {
      splitIndex: candidateIndex,
      turnStartIndex: -1,
      isSplitTurn: false,
    }
  }

  let turnStart = -1
  for (let i = candidateIndex - 1; i >= 0; i--) {
    if (messages[i].role === 'user') {
      turnStart = i
      break
    }
  }

  if (turnStart <= 0) {
    return {
      splitIndex: candidateIndex,
      turnStartIndex: -1,
      isSplitTurn: false,
    }
  }

  return {
    splitIndex: candidateIndex,
    turnStartIndex: turnStart,
    isSplitTurn: true,
  }
}

function formatTruncated(text: string, maxChars: number): string {
  return `${text.slice(0, maxChars)}\n\n[... truncated ${text.length - maxChars} characters]`
}

export function reduceToolOutputs(
  messages: ModelMessage[],
  options: OutputReductionOptions,
): ModelMessage[] {
  const keepRecentCount = options.keepRecentCount ?? 2
  const clearThreshold =
    options.clearThreshold ?? AGENT_LIMITS.COMPACTION_CLEAR_OUTPUT_MIN_CHARS
  const toolIndices: number[] = []

  for (let i = 0; i < messages.length; i++) {
    if (messages[i].role === 'tool') {
      toolIndices.push(i)
    }
  }

  const protectedIndices = new Set(
    keepRecentCount > 0 ? toolIndices.slice(-keepRecentCount) : [],
  )

  let cleared = 0
  let truncated = 0
  const reduced = messages.map((msg, idx) => {
    if (msg.role !== 'tool') return msg

    const content = msg.content.map((part) => {
      if (part.type !== 'tool-result') return part

      const output =
        part.output.type === 'content'
          ? stripToolResultOutput(part.output)
          : part.output
      const outputText = toolResultOutputToText(output)

      if (outputText.length <= clearThreshold) {
        return output === part.output ? part : { ...part, output }
      }

      if (protectedIndices.has(idx)) {
        if (outputText.length <= options.maxChars) {
          return output === part.output ? part : { ...part, output }
        }

        truncated++
        return {
          ...part,
          output: {
            type: 'text' as const,
            value: formatTruncated(outputText, options.maxChars),
          },
        }
      }

      cleared++
      return {
        ...part,
        output: {
          type: 'text' as const,
          value: `[Cleared — ${outputText.length} chars]`,
        },
      }
    })

    return { ...msg, content }
  })

  if (truncated > 0 || cleared > 0) {
    logger.info('Reduced tool outputs', {
      truncatedCount: truncated,
      clearedCount: cleared,
      protectedCount: protectedIndices.size,
    })
  }

  return reduced
}

export function slidingWindow(
  messages: ModelMessage[],
  maxTokens: number,
): ModelMessage[] {
  let totalTokens = estimateTokens(messages)
  let startIndex = 0

  while (totalTokens > maxTokens && startIndex < messages.length - 2) {
    const msg = messages[startIndex]

    if (msg.role === 'tool') {
      const nextMsg = messages[startIndex + 1]
      if (nextMsg?.role === 'assistant') {
        totalTokens -= estimateTokens([msg, nextMsg])
        startIndex += 2
        continue
      }
    }

    if (msg.role === 'assistant') {
      const nextMsg = messages[startIndex + 1]
      if (nextMsg?.role === 'tool') {
        totalTokens -= estimateTokens([msg, nextMsg])
        startIndex += 2
        continue
      }
    }

    totalTokens -= estimateTokens([msg])
    startIndex++
  }

  if (startIndex === 0) return messages

  logger.info('Sliding window applied', {
    droppedMessages: startIndex,
    remainingMessages: messages.length - startIndex,
    estimatedTokens: estimateTokens(messages.slice(startIndex)),
  })

  return messages.slice(startIndex)
}

export function isCompactionState(v: unknown): v is CompactionState {
  return (
    typeof v === 'object' &&
    v !== null &&
    'compactionCount' in v &&
    typeof (v as CompactionState).compactionCount === 'number'
  )
}
