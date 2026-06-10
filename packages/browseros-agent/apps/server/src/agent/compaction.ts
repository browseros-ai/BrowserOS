import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import {
  type LanguageModel,
  type ModelMessage,
  pruneMessages,
  streamText,
} from 'ai'
import { logger } from '../lib/logger'
import { stripBinaryContent } from './compaction/content'
import {
  buildSummarizationPrompt,
  buildSummarizationSystemPrompt,
  buildTurnPrefixPrompt,
  messagesToTranscript,
} from './compaction/prompt'
import {
  type CompactionState,
  type ComputedConfig,
  computeConfig,
  estimateTokens,
  estimateTokensForThreshold,
  findSafeSplitPoint,
  getCurrentTokenCount,
  isCompactionState,
  reduceToolOutputs,
  type StepWithUsage,
  slidingWindow,
} from './compaction/utils'

export {
  type CompactionState,
  computeConfig,
  estimateTokens,
  estimateTokensForThreshold,
  findSafeSplitPoint,
  getCurrentTokenCount,
  reduceToolOutputs,
  type StepWithUsage,
  slidingWindow,
} from './compaction/utils'

export interface CompactionConfig {
  contextWindow: number
}

async function consumeStreamText(
  result: ReturnType<typeof streamText>,
): Promise<string> {
  const chunks: string[] = []
  for await (const chunk of result.textStream) {
    chunks.push(chunk)
  }
  return chunks.join('')
}

async function callSummarizer(
  model: LanguageModel,
  messages: ModelMessage[],
  userPrompt: string,
  timeoutMs: number,
  maxOutputTokens: number,
  logLabel: string,
): Promise<string | null> {
  const transcript = messagesToTranscript(messages)
  if (!transcript.trim()) return null

  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), timeoutMs)

  try {
    const result = streamText({
      model,
      system: buildSummarizationSystemPrompt(),
      maxOutputTokens,
      messages: [
        {
          role: 'user',
          content: `<conversation_transcript>\n${transcript}\n</conversation_transcript>\n\n${userPrompt}`,
        },
      ],
      abortSignal: controller.signal,
    })

    const text = await consumeStreamText(result)
    return text || null
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    logger.warn(`${logLabel} failed`, { error: message })
    return null
  } finally {
    clearTimeout(timeout)
  }
}

async function summarizeMessages(
  model: LanguageModel,
  messagesToSummarize: ModelMessage[],
  existingSummary: string | null,
  timeoutMs: number,
  maxOutputTokens: number,
): Promise<string | null> {
  return callSummarizer(
    model,
    messagesToSummarize,
    buildSummarizationPrompt(existingSummary),
    timeoutMs,
    maxOutputTokens,
    'Summarization',
  )
}

async function summarizeTurnPrefix(
  model: LanguageModel,
  turnPrefixMessages: ModelMessage[],
  timeoutMs: number,
  maxOutputTokens: number,
): Promise<string | null> {
  return callSummarizer(
    model,
    turnPrefixMessages,
    buildTurnPrefixPrompt(),
    timeoutMs,
    maxOutputTokens,
    'Turn prefix summarization',
  )
}

async function compactMessages(
  model: LanguageModel,
  messages: ModelMessage[],
  config: ComputedConfig,
  state: CompactionState,
): Promise<ModelMessage[]> {
  const { splitIndex, turnStartIndex, isSplitTurn } = findSafeSplitPoint(
    messages,
    config.keepRecentTokens,
    config.imageTokenEstimate,
  )

  if (splitIndex === -1) {
    logger.info('Cannot find safe split point, using sliding window')
    return slidingWindow(messages, config.triggerThreshold)
  }

  const toKeep = messages.slice(splitIndex)
  let historyMessages: ModelMessage[]
  let turnPrefixMessages: ModelMessage[] = []

  if (isSplitTurn && turnStartIndex >= 0) {
    historyMessages = messages.slice(0, turnStartIndex)
    turnPrefixMessages = messages.slice(turnStartIndex, splitIndex)
    logger.info('Split turn detected', {
      historyMessages: historyMessages.length,
      turnPrefixMessages: turnPrefixMessages.length,
      toKeepMessages: toKeep.length,
    })
  } else {
    historyMessages = messages.slice(0, splitIndex)
  }

  let toSummarize = historyMessages
  let summarizedTurnPrefix = turnPrefixMessages

  if (toSummarize.length > 0) {
    const summarizeTokens = estimateTokens(toSummarize)
    if (summarizeTokens > config.maxSummarizationInput) {
      logger.info('Capping summarization input, dropping oldest messages', {
        excess: summarizeTokens - config.maxSummarizationInput,
        maxSummarizationInput: config.maxSummarizationInput,
      })
      toSummarize = slidingWindow(toSummarize, config.maxSummarizationInput)
    }
  }

  if (summarizedTurnPrefix.length > 0) {
    const prefixTokens = estimateTokens(summarizedTurnPrefix)
    if (prefixTokens > config.maxSummarizationInput) {
      logger.info('Capping turn prefix input, dropping oldest messages', {
        excess: prefixTokens - config.maxSummarizationInput,
        maxSummarizationInput: config.maxSummarizationInput,
      })
      summarizedTurnPrefix = slidingWindow(
        summarizedTurnPrefix,
        config.maxSummarizationInput,
      )
    }
  }

  const totalSummarizable =
    estimateTokens(toSummarize) + estimateTokens(summarizedTurnPrefix)
  if (totalSummarizable < config.minSummarizableTokens) {
    logger.info('Too little content to summarize, using sliding window')
    return slidingWindow(messages, config.triggerThreshold)
  }

  const turnPrefixOutputBudget = Math.max(
    AGENT_LIMITS.COMPACTION_MIN_TOKEN_FLOOR,
    Math.floor(
      config.summarizerMaxOutputTokens *
        AGENT_LIMITS.COMPACTION_TURN_PREFIX_OUTPUT_RATIO,
    ),
  )

  logger.info('Attempting LLM-based compaction', {
    toSummarizeMessages: toSummarize.length,
    toSummarizeTokens: estimateTokens(toSummarize),
    turnPrefixMessages: summarizedTurnPrefix.length,
    turnPrefixTokens: estimateTokens(summarizedTurnPrefix),
    toKeepMessages: toKeep.length,
    toKeepTokens: estimateTokens(toKeep),
    isSplitTurn,
    hasExistingSummary: state.existingSummary != null,
    compactionCount: state.compactionCount,
  })

  let summary: string | null = null
  if (isSplitTurn && summarizedTurnPrefix.length > 0) {
    if (toSummarize.length > 0) {
      const [historySummary, turnPrefixSummary] = await Promise.all([
        summarizeMessages(
          model,
          toSummarize,
          state.existingSummary,
          config.summarizationTimeoutMs,
          config.summarizerMaxOutputTokens,
        ),
        summarizeTurnPrefix(
          model,
          summarizedTurnPrefix,
          config.summarizationTimeoutMs,
          turnPrefixOutputBudget,
        ),
      ])

      if (historySummary && turnPrefixSummary) {
        summary = `${historySummary}\n\n---\n\n**Turn Context (split turn):**\n\n${turnPrefixSummary}`
      } else {
        summary = historySummary ?? turnPrefixSummary
      }
    } else {
      summary = await summarizeTurnPrefix(
        model,
        summarizedTurnPrefix,
        config.summarizationTimeoutMs,
        turnPrefixOutputBudget,
      )
    }
  } else {
    summary = await summarizeMessages(
      model,
      toSummarize,
      state.existingSummary,
      config.summarizationTimeoutMs,
      config.summarizerMaxOutputTokens,
    )
  }

  if (!summary) {
    logger.warn('Summarization returned empty, using sliding window fallback')
    return slidingWindow(messages, config.triggerThreshold)
  }

  const allSummarized = [...toSummarize, ...summarizedTurnPrefix]
  const summaryTokens = Math.ceil(summary.length / 4)
  const originalTokens = estimateTokens(allSummarized)
  if (summaryTokens >= originalTokens) {
    logger.warn(
      'Summary is larger than original, using sliding window fallback',
      {
        summaryTokens,
        originalTokens,
      },
    )
    return slidingWindow(messages, config.triggerThreshold)
  }

  state.existingSummary = summary
  state.compactionCount++

  logger.info('LLM compaction succeeded', {
    originalMessages: messages.length,
    keptMessages: toKeep.length,
    summaryTokens,
    originalTokens,
    compressionRatio: `${((1 - summaryTokens / originalTokens) * 100).toFixed(0)}%`,
    compactionCount: state.compactionCount,
    isSplitTurn,
  })

  return [
    {
      role: 'user',
      content: `${summary}\n\nContinue from where you left off.`,
    },
    ...toKeep,
  ]
}

export function createCompactionPrepareStep(
  userConfig?: Partial<CompactionConfig>,
) {
  const contextWindow =
    userConfig?.contextWindow ?? AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW
  const config = computeConfig(contextWindow)

  logger.info('Compaction config computed', {
    contextWindow,
    reserveTokens: config.reserveTokens,
    triggerRatio: config.triggerRatio.toFixed(3),
    triggerAtTokens: Math.floor(config.triggerThreshold),
    keepRecentTokens: config.keepRecentTokens,
    minSummarizableTokens: config.minSummarizableTokens,
    maxSummarizationInput: config.maxSummarizationInput,
    summarizerMaxOutputTokens: config.summarizerMaxOutputTokens,
  })

  return async ({
    messages,
    steps,
    model,
    experimental_context,
  }: {
    messages: ModelMessage[]
    steps: ReadonlyArray<StepWithUsage>
    model: LanguageModel
    experimental_context: unknown
  }) => {
    const state: CompactionState = isCompactionState(experimental_context)
      ? experimental_context
      : { existingSummary: null, compactionCount: 0 }

    let currentTokens = getCurrentTokenCount(steps, messages, config)
    if (currentTokens <= config.triggerThreshold) {
      return { messages, experimental_context: state }
    }

    let current = stripBinaryContent(messages)
    currentTokens = estimateTokensForThreshold(current, config)
    if (currentTokens <= config.triggerThreshold) {
      return { messages: current, experimental_context: state }
    }

    const keepRecent = AGENT_LIMITS.COMPACTION_PRUNE_KEEP_RECENT_MESSAGES
    const pruned = pruneMessages({
      messages: current,
      toolCalls: `before-last-${keepRecent}-messages`,
      emptyMessages: 'remove',
    })
    if (pruned.length < current.length) {
      logger.info('Pruned old tool calls', {
        before: current.length,
        after: pruned.length,
        removed: current.length - pruned.length,
      })
      current = pruned
      currentTokens = estimateTokensForThreshold(current, config)
      if (currentTokens <= config.triggerThreshold) {
        return { messages: current, experimental_context: state }
      }
    }

    const reduced = reduceToolOutputs(current, {
      maxChars: config.toolOutputMaxChars,
      keepRecentCount: 2,
    })
    currentTokens = estimateTokensForThreshold(reduced, config)
    if (currentTokens <= config.triggerThreshold) {
      return { messages: reduced, experimental_context: state }
    }

    logger.warn(
      'Context still over limit after output reduction, attempting compaction',
      {
        currentTokens,
        triggerThreshold: Math.floor(config.triggerThreshold),
        messageCount: reduced.length,
      },
    )

    const compacted = await compactMessages(model, reduced, config, state)
    return { messages: compacted, experimental_context: state }
  }
}
