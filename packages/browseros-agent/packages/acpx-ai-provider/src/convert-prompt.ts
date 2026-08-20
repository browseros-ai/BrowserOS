import type {
  LanguageModelV2CallOptions,
  LanguageModelV2FilePart,
  LanguageModelV2Message,
  LanguageModelV2Prompt,
  LanguageModelV2ReasoningPart,
  LanguageModelV2TextPart,
  LanguageModelV2ToolCallPart,
  LanguageModelV2ToolResultOutput,
  LanguageModelV2ToolResultPart,
} from '@ai-sdk/provider'
import {
  convertBase64ToUint8Array,
  convertUint8ArrayToBase64,
} from '@ai-sdk/provider-utils'

export type ConvertPromptMode = 'fresh' | 'continuation'

export interface ConvertPromptInput {
  prompt: LanguageModelV2Prompt
  responseFormat?: LanguageModelV2CallOptions['responseFormat']
  mode: ConvertPromptMode
}

export interface ConvertPromptAttachment {
  mediaType: string
  data: string
}

export interface ConvertPromptOutput {
  text: string
  attachments: ConvertPromptAttachment[]
}

const ROLE_PREFIX: Record<LanguageModelV2Message['role'], string> = {
  system: 'System: ',
  user: 'User: ',
  assistant: 'Assistant: ',
  tool: 'Tool: ',
}

const FILE_URL_NOT_SUPPORTED =
  'convertPrompt does not support remote URL file parts; inline the file as base64 or Uint8Array before passing it to the provider.'

type JsonResponseFormat = Extract<
  NonNullable<LanguageModelV2CallOptions['responseFormat']>,
  { type: 'json' }
>

type AssistantContentPart =
  | LanguageModelV2TextPart
  | LanguageModelV2FilePart
  | LanguageModelV2ReasoningPart
  | LanguageModelV2ToolCallPart
  | LanguageModelV2ToolResultPart

export function convertPrompt(input: ConvertPromptInput): ConvertPromptOutput {
  const { prompt, responseFormat, mode } = input
  const messages = filterMessagesForMode(prompt, mode)

  const lines: string[] = []
  const attachments: ConvertPromptAttachment[] = []

  if (responseFormat?.type === 'json') {
    lines.push(buildJsonSchemaPrompt(responseFormat))
  }

  for (const message of messages) {
    appendMessage(message, lines, attachments)
  }

  return { text: lines.join('\n'), attachments }
}

function filterMessagesForMode(
  prompt: LanguageModelV2Prompt,
  mode: ConvertPromptMode,
): LanguageModelV2Prompt {
  if (mode === 'fresh') return prompt
  for (let i = prompt.length - 1; i >= 0; i -= 1) {
    const message = prompt[i]
    if (message?.role === 'user') return [message]
  }
  return []
}

function appendMessage(
  message: LanguageModelV2Message,
  lines: string[],
  attachments: ConvertPromptAttachment[],
): void {
  const prefix = ROLE_PREFIX[message.role]

  if (message.role === 'system') {
    lines.push(`${prefix}${message.content}`)
    return
  }

  const segments: string[] = []
  for (const part of message.content as AssistantContentPart[]) {
    const fragment = renderPart(part, attachments)
    if (fragment !== undefined) segments.push(fragment)
  }
  if (segments.length === 0) return
  lines.push(`${prefix}${segments.join(' ')}`)
}

function renderPart(
  part: AssistantContentPart,
  attachments: ConvertPromptAttachment[],
): string | undefined {
  switch (part.type) {
    case 'text':
      return part.text
    case 'reasoning':
      return `[Reasoning: ${part.text}]`
    case 'tool-call':
      return `[Tool call: ${part.toolName}(${JSON.stringify(part.input)})]`
    case 'tool-result':
      return `[Tool result (${part.toolCallId}): ${formatToolOutput(part.output)}]`
    case 'file':
      return renderFile(part, attachments)
  }
}

function renderFile(
  part: LanguageModelV2FilePart,
  attachments: ConvertPromptAttachment[],
): string | undefined {
  if (
    part.mediaType.startsWith('image/') ||
    part.mediaType.startsWith('audio/')
  ) {
    attachments.push(toAttachment(part))
    return undefined
  }
  if (
    part.mediaType.startsWith('text/') ||
    part.mediaType === 'application/json'
  ) {
    const label = part.filename ?? part.mediaType
    return `[File: ${label}]\n${decodeTextFile(part)}`
  }
  throw new Error(`Unsupported ACP file type: ${part.mediaType}`)
}

function formatToolOutput(output: LanguageModelV2ToolResultOutput): string {
  switch (output.type) {
    case 'text':
    case 'error-text':
      return output.value
    case 'json':
    case 'error-json':
      return JSON.stringify(output.value)
    case 'content':
      return output.value
        .map((item) =>
          item.type === 'text' ? item.text : `<media:${item.mediaType}>`,
        )
        .join(' ')
  }
}

/**
 * v7 hands file-part `data` as a tagged `FileData` object
 * (`{ type: 'data' | 'url' | 'text', ... }`) even though the V2 part type
 * still says `string | Uint8Array | URL`. Normalize both shapes to the raw
 * value the callers below already understand.
 */
function unwrapFileData(raw: unknown): string | Uint8Array | URL {
  if (
    raw instanceof URL ||
    raw instanceof Uint8Array ||
    typeof raw === 'string'
  ) {
    return raw
  }
  if (raw && typeof raw === 'object' && 'type' in raw) {
    const fd = raw as {
      type: string
      data?: string | Uint8Array
      url?: URL | string
      text?: string
    }
    if (fd.type === 'data' && fd.data != null) return fd.data
    if (fd.type === 'url' && fd.url != null) {
      return fd.url instanceof URL ? fd.url : new URL(String(fd.url))
    }
    if (fd.type === 'text') {
      return `data:text/plain;charset=utf-8,${encodeURIComponent(fd.text ?? '')}`
    }
  }
  throw new Error(FILE_URL_NOT_SUPPORTED)
}

function toAttachment(part: LanguageModelV2FilePart): ConvertPromptAttachment {
  const data = unwrapFileData(part.data)
  if (data instanceof URL) {
    if (data.protocol !== 'data:') throw new Error(FILE_URL_NOT_SUPPORTED)
    return { mediaType: part.mediaType, data: extractBase64Data(data.href) }
  }
  if (typeof data === 'string') {
    return { mediaType: part.mediaType, data: extractBase64Data(data) }
  }
  if (data instanceof Uint8Array) {
    return { mediaType: part.mediaType, data: convertUint8ArrayToBase64(data) }
  }
  throw new Error(FILE_URL_NOT_SUPPORTED)
}

function extractBase64Data(value: string): string {
  if (!value.startsWith('data:')) return value
  const commaIndex = value.indexOf(',')
  return commaIndex >= 0 ? value.slice(commaIndex + 1) : value
}

function decodeTextFile(part: LanguageModelV2FilePart): string {
  let data = unwrapFileData(part.data)
  if (data instanceof URL) {
    if (data.protocol !== 'data:') throw new Error(FILE_URL_NOT_SUPPORTED)
    data = data.href
  }
  if (data instanceof Uint8Array) {
    return new TextDecoder().decode(data)
  }
  if (data.startsWith('data:')) {
    const commaIndex = data.indexOf(',')
    if (commaIndex < 0) return ''
    const metadata = data.slice(0, commaIndex)
    const value = data.slice(commaIndex + 1)
    if (!metadata.endsWith(';base64')) return decodeURIComponent(value)
    return new TextDecoder().decode(convertBase64ToUint8Array(value))
  }
  return new TextDecoder().decode(convertBase64ToUint8Array(data))
}

function buildJsonSchemaPrompt(responseFormat: JsonResponseFormat): string {
  const parts = [
    '[Structured Output Instruction]',
    'You MUST respond with a single valid JSON value.',
    'Do NOT wrap JSON in markdown fences (no ```json blocks).',
    'Do NOT add explanations, comments, or any other text before or after the JSON.',
    'Your entire response must be ONLY the JSON value, nothing else.',
  ]
  if (responseFormat.name) {
    parts.push(`Output name: ${responseFormat.name}`)
  }
  if (responseFormat.description) {
    parts.push(`Output description: ${responseFormat.description}`)
  }
  if (responseFormat.schema) {
    parts.push(
      'The JSON value MUST conform to this JSON Schema:',
      JSON.stringify(responseFormat.schema, null, 2),
    )
  }
  parts.push('[End Structured Output Instruction]')
  return parts.join('\n')
}
