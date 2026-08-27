/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import type {
  AssistantContent,
  ModelMessage,
  ToolContent,
  UserContent,
} from 'ai'
import { estimateToolResultOutput } from './content'

export interface StepWithUsage {
  usage?: {
    inputTokens?: number | undefined
    outputTokens?: number | undefined
  }
}

/**
 * Characters per token. The AI SDK compaction guide suggests 4, but browser
 * transcripts are dense with markup and JSON tool arguments, which tokenize
 * closer to 3 characters per token.
 */
const CHARS_PER_TOKEN = 3

function safeJsonStringify(value: unknown): string {
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
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

/**
 * Estimates tokens for a message list.
 *
 * Images are counted as a flat per-image cost rather than by their encoded
 * length, so a base64 screenshot contributes its render cost instead of
 * hundreds of thousands of characters. This is why the guide's
 * `JSON.stringify(messages).length / 4` is not usable here.
 */
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

  return Math.ceil(chars / CHARS_PER_TOKEN) + imageCount * imageTokenEstimate
}

/**
 * Estimated prompt size, including the overhead `prepareStep` cannot see.
 *
 * `prepareStep` receives only the conversation messages: the system prompt and
 * the tool schemas are passed separately and are invisible here, so estimating
 * from messages alone under-counts every request by a five-figure token amount.
 */
export function estimateTotalTokens(
  messages: ModelMessage[],
  overheadTokens: number,
): number {
  return estimateTokens(messages) + overheadTokens
}

/**
 * Current prompt size, preferring the provider's own accounting.
 *
 * When the previous step reported usage, that number already includes the
 * system prompt and tool schemas. Messages appended after that reading (a fresh
 * page snapshot, for example) are not in it, so they are estimated and added.
 */
export function getCurrentTokenCount(
  steps: ReadonlyArray<StepWithUsage>,
  messages: ModelMessage[],
  overheadTokens: number,
): number {
  const lastStep = steps.at(-1)
  const reportedInput = lastStep?.usage?.inputTokens

  if (reportedInput == null || reportedInput <= 0) {
    return estimateTotalTokens(messages, overheadTokens)
  }

  const base = reportedInput + (lastStep?.usage?.outputTokens ?? 0)
  const lastAssistantIndex = messages.findLastIndex(
    (message) => message.role === 'assistant',
  )
  if (lastAssistantIndex === -1) {
    return base
  }

  let trailingTokens = 0
  for (let i = messages.length - 1; i > lastAssistantIndex; i--) {
    if (messages[i].role === 'system') continue
    trailingTokens += estimateTokens([messages[i]])
  }

  return base + trailingTokens
}
