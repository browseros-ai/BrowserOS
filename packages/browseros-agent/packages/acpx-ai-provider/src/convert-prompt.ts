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
import { convertUint8ArrayToBase64 } from '@ai-sdk/provider-utils'

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
      attachments.push(toAttachment(part))
      return undefined
  }
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

function toAttachment(part: LanguageModelV2FilePart): ConvertPromptAttachment {
  const data = part.data
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
