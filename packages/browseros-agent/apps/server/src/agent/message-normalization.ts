import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import type {
  FilePart,
  ImagePart,
  ModelMessage,
  ToolModelMessage,
  ToolResultPart,
  UserContent,
  UserModelMessage,
} from 'ai'
import { stripToolResultOutput } from './compaction/content'
import type { ResolvedAgentConfig } from './types'

type ToolResultOutput = ToolResultPart['output']
type ToolResultContentPart = Extract<
  ToolResultOutput,
  { type: 'content' }
>['value'][number]
type UserMessagePart = Exclude<UserContent, string>[number]
type UserMediaPart = Extract<UserMessagePart, ImagePart | FilePart>

export interface MessageNormalizationOptions {
  supportsImages: boolean
  supportsMediaInToolResults: boolean
}

// See how opencode handles, inspiration from there
// https://github.com/anomalyco/opencode/blob/5ec5d1daceaab23c8ffa9ae32b40f53120f4609e/packages/opencode/src/session/message-v2.ts#L503-L522
function supportsToolResultMediaTransport(
  config: ResolvedAgentConfig,
): boolean {
  switch (config.provider) {
    case LLM_PROVIDERS.ANTHROPIC:
    case LLM_PROVIDERS.OPENAI:
    case LLM_PROVIDERS.AZURE:
    case LLM_PROVIDERS.BEDROCK:
      return true
    case LLM_PROVIDERS.GOOGLE: {
      const modelId = config.model.toLowerCase()
      return modelId.includes('gemini-3') && !modelId.includes('gemini-2')
    }
    default:
      return false
  }
}

export function getMessageNormalizationOptions(
  config: ResolvedAgentConfig,
): MessageNormalizationOptions {
  return {
    supportsImages: config.supportsImages !== false,
    supportsMediaInToolResults: supportsToolResultMediaTransport(config),
  }
}

function buildToolResultMediaLabel(parts: UserMediaPart[]): string {
  const imageCount = parts.filter(
    (part) =>
      part.type === 'image' ||
      (part.type === 'file' && part.mediaType.startsWith('image/')),
  ).length

  if (imageCount === parts.length) {
    return 'Attached image(s) from tool result:'
  }

  if (imageCount === 0) {
    return 'Attached file(s) from tool result:'
  }

  return 'Attached files from tool result:'
}

function toolResultContentPartToUserMedia(
  part: ToolResultContentPart,
): UserMediaPart | null {
  switch (part.type) {
    // The canonical AI SDK v7 file part is what the MCP client actually emits
    // for an image tool result (image-data/file-data are its deprecated
    // predecessors). Its data is a tagged union; only the inline `data` variant
    // carries bytes we can re-attach. Without this case a screenshot is dropped
    // for every provider that cannot carry media inside a tool result (#2722).
    case 'file': {
      if (part.data.type !== 'data') {
        return null
      }
      if (part.mediaType.startsWith('image/')) {
        return {
          type: 'image',
          image: part.data.data,
          mediaType: part.mediaType,
        }
      }
      return {
        type: 'file',
        data: part.data.data,
        mediaType: part.mediaType,
        filename: part.filename,
      }
    }
    case 'image-data':
      if (part.mediaType.startsWith('image/')) {
        return {
          type: 'image',
          image: part.data,
          mediaType: part.mediaType,
        }
      }
      return {
        type: 'file',
        data: part.data,
        mediaType: part.mediaType,
      }
    case 'file-data':
      if (part.mediaType.startsWith('image/')) {
        return {
          type: 'image',
          image: part.data,
          mediaType: part.mediaType,
        }
      }
      return {
        type: 'file',
        data: part.data,
        mediaType: part.mediaType,
        filename: part.filename,
      }
    default:
      return null
  }
}

function normalizeToolMessageForModel(
  message: ToolModelMessage,
  supportsImages: boolean,
): ModelMessage[] {
  let extractedMedia: UserMediaPart[] = []
  let changed = false

  const content = message.content.map((part) => {
    if (part.type !== 'tool-result' || part.output.type !== 'content') {
      return part
    }

    changed = true

    if (supportsImages) {
      extractedMedia = [
        ...extractedMedia,
        ...part.output.value
          .map(toolResultContentPartToUserMedia)
          .filter(
            (mediaPart): mediaPart is UserMediaPart => mediaPart !== null,
          ),
      ]
    }

    return {
      ...part,
      output: stripToolResultOutput(part.output),
    }
  })

  if (!changed) {
    return [message]
  }

  const normalizedToolMessage: ToolModelMessage = {
    ...message,
    content,
  }

  if (extractedMedia.length === 0) {
    return [normalizedToolMessage]
  }

  const mediaMessage: UserModelMessage = {
    role: 'user',
    content: [
      {
        type: 'text',
        text: buildToolResultMediaLabel(extractedMedia),
      },
      ...extractedMedia,
    ],
  }

  return [normalizedToolMessage, mediaMessage]
}

// opencode handles this at message serialization time for providers that do not
// reliably support media inside tool results. BrowserOS uses the same boundary:
// normalize model messages before the next step, not inside the screenshot tool.
export function normalizeMessagesForModel(
  messages: ModelMessage[],
  options: MessageNormalizationOptions,
): ModelMessage[] {
  if (options.supportsMediaInToolResults) {
    return messages
  }

  let changed = false
  const normalized: ModelMessage[] = []

  for (const message of messages) {
    if (message.role !== 'tool') {
      normalized.push(message)
      continue
    }

    const replacement = normalizeToolMessageForModel(
      message,
      options.supportsImages,
    )
    if (replacement.length !== 1 || replacement[0] !== message) {
      changed = true
    }
    normalized.push(...replacement)
  }

  return changed ? normalized : messages
}

// The @ai-sdk/openai-compatible serializer re-emits an assistant reasoning part
// as a top-level `reasoning_content` field on the request. `reasoning_content`
// is response-only in the chat-completions schema, so strict gateways (Mistral,
// Cerebras, Fireworks) reject a replayed one with 400/422. stripReasoningParts
// already removes reasoning from the pre-run UIMessage history, but the assistant
// turn a multi-step tool loop generates mid-run never passes through that path:
// it is built from the running steps and replayed on the next step. Strip it here
// at the model-message boundary, which prepareStep runs on every step. Reasoning
// stays in the durable, client-displayed history; only the replayed copy is
// cleaned, and the model generates fresh reasoning for the new step.
export function stripReasoningFromModelMessages(
  messages: ModelMessage[],
): ModelMessage[] {
  let changed = false
  const result: ModelMessage[] = []

  for (const message of messages) {
    if (message.role !== 'assistant') {
      result.push(message)
      continue
    }
    if (typeof message.content === 'string') {
      result.push(message)
      continue
    }
    const content = message.content.filter((part) => part.type !== 'reasoning')
    if (content.length === message.content.length) {
      result.push(message)
      continue
    }
    changed = true
    // A reasoning-only assistant turn (no text, no tool call) would serialize to
    // an empty assistant message, which strict providers also reject; drop it
    // rather than replay an empty turn.
    if (content.length === 0) continue
    result.push({ ...message, content })
  }

  return changed ? result : messages
}
