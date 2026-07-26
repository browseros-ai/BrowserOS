import type {
  AssistantContent,
  AssistantModelMessage,
  ModelMessage,
  ToolModelMessage,
  ToolResultPart,
  UserContent,
  UserModelMessage,
} from 'ai'

export type ToolResultOutput = ToolResultPart['output']
type ToolResultContentPart = Extract<
  ToolResultOutput,
  { type: 'content' }
>['value'][number]

function safeJsonStringify(value: unknown): string {
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

function formatExecutionDenied(reason?: string): string {
  return reason ? `[Execution denied: ${reason}]` : '[Execution denied]'
}

function formatFileId(fileId: string | Record<string, string>): string {
  return typeof fileId === 'string' ? fileId : safeJsonStringify(fileId)
}

function formatFilePlaceholder(mediaType?: string, filename?: string): string {
  if (mediaType?.startsWith('image/')) {
    return '[Image]'
  }
  return filename ? `[File: ${filename}]` : '[File]'
}

function isBinaryToolResultContentPart(part: ToolResultContentPart): boolean {
  return (
    part.type === 'media' ||
    part.type === 'image-data' ||
    part.type === 'file-data'
  )
}

function toolResultContentPartToText(part: ToolResultContentPart): string {
  switch (part.type) {
    case 'text':
      return part.text
    case 'media':
    case 'image-data':
      return '[Image]'
    case 'file-data':
      return part.filename ? `[File: ${part.filename}]` : '[File]'
    case 'file-url':
    case 'image-url':
      return part.url
    case 'file-id':
    case 'image-file-id':
      return formatFileId(part.fileId)
    case 'custom':
      return '[Custom content]'
  }
}

function toolResultContentToText(parts: ToolResultContentPart[]): string {
  return parts
    .map(toolResultContentPartToText)
    .filter((part) => part.length > 0)
    .join('\n')
}

export function toolResultOutputToText(output: ToolResultOutput): string {
  switch (output.type) {
    case 'text':
    case 'error-text':
      return output.value
    case 'json':
    case 'error-json':
      return safeJsonStringify(output.value)
    case 'execution-denied':
      return formatExecutionDenied(output.reason)
    case 'content':
      return toolResultContentToText(output.value)
  }
}

export function estimateToolResultOutput(output: ToolResultOutput): {
  chars: number
  images: number
} {
  switch (output.type) {
    case 'text':
    case 'error-text':
      return { chars: output.value.length, images: 0 }
    case 'json':
    case 'error-json':
      return { chars: safeJsonStringify(output.value).length, images: 0 }
    case 'execution-denied':
      return { chars: formatExecutionDenied(output.reason).length, images: 0 }
    case 'content': {
      let chars = 0
      let images = 0
      for (const part of output.value) {
        switch (part.type) {
          case 'text':
            chars += part.text.length
            break
          case 'media':
          case 'image-data':
          case 'file-data':
            images++
            break
          case 'file-url':
          case 'image-url':
            chars += part.url.length
            break
          case 'file-id':
          case 'image-file-id':
            chars += formatFileId(part.fileId).length
            break
          case 'custom':
            break
        }
      }
      return { chars, images }
    }
  }
}

export function stripToolResultOutput(
  output: ToolResultOutput,
): ToolResultOutput {
  if (output.type !== 'content') return output

  const text = toolResultContentToText(output.value)
  return {
    type: 'text',
    value: text || '[Tool content omitted]',
  }
}

function stripToolMessage(msg: ToolModelMessage): ToolModelMessage {
  return {
    ...msg,
    content: msg.content.map((part) =>
      part.type !== 'tool-result'
        ? part
        : { ...part, output: stripToolResultOutput(part.output) },
    ),
  }
}

function stripUserContent(content: UserContent): UserContent {
  if (typeof content === 'string') return content

  return content.map((part) => {
    if (part.type === 'image') {
      return { type: 'text' as const, text: '[Image]' }
    }
    if (part.type === 'file') {
      return {
        type: 'text' as const,
        text: formatFilePlaceholder(part.mediaType, part.filename),
      }
    }
    return part
  })
}

function stripUserMessage(msg: UserModelMessage): UserModelMessage {
  return {
    ...msg,
    content: stripUserContent(msg.content),
  }
}

function stripAssistantContent(content: AssistantContent): AssistantContent {
  if (typeof content === 'string') return content

  return content.map((part) => {
    if (part.type === 'file') {
      return {
        type: 'text' as const,
        text: formatFilePlaceholder(part.mediaType, part.filename),
      }
    }
    if (part.type === 'tool-result') {
      return { ...part, output: stripToolResultOutput(part.output) }
    }
    return part
  })
}

function stripAssistantMessage(
  msg: AssistantModelMessage,
): AssistantModelMessage {
  return {
    ...msg,
    content: stripAssistantContent(msg.content),
  }
}

export function stripBinaryContent(messages: ModelMessage[]): ModelMessage[] {
  return messages.map((msg) => {
    switch (msg.role) {
      case 'user':
        return stripUserMessage(msg)
      case 'assistant':
        return stripAssistantMessage(msg)
      case 'tool':
        return stripToolMessage(msg)
      case 'system':
        return msg
      default:
        return msg
    }
  })
}

export function countBinaryParts(messages: ModelMessage[]): number {
  let count = 0

  for (const msg of messages) {
    if (msg.role !== 'tool') continue

    for (const part of msg.content) {
      if (part.type !== 'tool-result' || part.output.type !== 'content')
        continue

      for (const contentPart of part.output.value) {
        if (isBinaryToolResultContentPart(contentPart)) count++
      }
    }
  }

  return count
}
