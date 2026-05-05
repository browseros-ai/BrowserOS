/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import type { AcpSessionRecord, createRuntimeStore } from 'acpx/runtime'
import type {
  OpenAIChatMessage,
  OpenAIContentPart,
} from '../../../api/services/openclaw/openclaw-gateway-chat-client'
import { logger } from '../../logger'
import type { AcpxAdapterTurnInput } from '../acpx-agent-adapter'
import type { AgentStreamEvent } from '../types'

type ImageAttachment = Readonly<{ mediaType: string; data: string }>

export async function maybeHandleOpenClawTurn(
  input: AcpxAdapterTurnInput,
): Promise<ReadableStream<AgentStreamEvent> | null> {
  const imageAttachments = (input.prompt.attachments ?? []).filter((a) =>
    a.mediaType.startsWith('image/'),
  )
  if (imageAttachments.length === 0 || !input.openclawGatewayChat) {
    return null
  }
  return sendOpenclawViaGateway({
    prompt: input.prompt,
    sessionStore: input.sessionStore,
    openclawGatewayChat: input.openclawGatewayChat,
    imageAttachments,
    cwd: input.prepared.cwd,
    runPrompt: input.prepared.runPrompt,
  })
}

/** Handles OpenClaw image turns through the gateway HTTP chat endpoint. */
async function sendOpenclawViaGateway(input: {
  prompt: AcpxAdapterTurnInput['prompt']
  sessionStore: AcpxAdapterTurnInput['sessionStore']
  openclawGatewayChat: NonNullable<AcpxAdapterTurnInput['openclawGatewayChat']>
  imageAttachments: ReadonlyArray<ImageAttachment>
  cwd: string
  runPrompt: string
}): Promise<ReadableStream<AgentStreamEvent>> {
  const existingRecord = await input.sessionStore.load(input.prompt.sessionKey)
  const priorMessages = existingRecord
    ? recordToOpenAIMessages(existingRecord)
    : []
  const userContent: OpenAIContentPart[] = [
    {
      type: 'text',
      text: input.runPrompt,
    },
    ...input.imageAttachments.map(
      (a): OpenAIContentPart => ({
        type: 'image_url',
        image_url: { url: `data:${a.mediaType};base64,${a.data}` },
      }),
    ),
  ]
  const messages: OpenAIChatMessage[] = [
    ...priorMessages,
    { role: 'user', content: userContent },
  ]

  logger.info('Agent harness gateway image turn dispatched', {
    agentId: input.prompt.agent.id,
    sessionKey: input.prompt.sessionKey,
    cwd: input.cwd,
    priorMessageCount: priorMessages.length,
    imageAttachmentCount: input.imageAttachments.length,
  })

  const upstream = await input.openclawGatewayChat.streamTurn({
    agentId: input.prompt.agent.id,
    sessionKey: input.prompt.sessionKey,
    messages,
    signal: input.prompt.signal,
  })

  const sessionStore = input.sessionStore
  const sessionKey = input.prompt.sessionKey
  const userMessageText = input.prompt.message
  const imageAttachments = input.imageAttachments
  let accumulated = ''

  return new ReadableStream<AgentStreamEvent>({
    start: (controller) => {
      const reader = upstream.getReader()
      const persist = async () => {
        if (!existingRecord || !accumulated) return
        try {
          await persistGatewayTurn(
            sessionStore,
            sessionKey,
            userMessageText,
            imageAttachments,
            accumulated,
          )
        } catch (err) {
          logger.warn(
            'Failed to persist gateway image turn to acpx session record',
            {
              sessionKey,
              error: err instanceof Error ? err.message : String(err),
            },
          )
        }
      }
      ;(async () => {
        try {
          while (true) {
            const { done, value } = await reader.read()
            if (done) break
            if (value.type === 'text_delta') accumulated += value.text
            controller.enqueue(value)
          }
          await persist()
          controller.close()
        } catch (err) {
          controller.enqueue({
            type: 'error',
            message: err instanceof Error ? err.message : String(err),
          })
          controller.close()
        }
      })().catch(() => {})
    },
    cancel: () => {
      // Best-effort: cancel propagation to the gateway is tracked separately.
    },
  })
}

async function persistGatewayTurn(
  sessionStore: ReturnType<typeof createRuntimeStore>,
  sessionKey: string,
  userMessageText: string,
  imageAttachments: ReadonlyArray<ImageAttachment>,
  assistantText: string,
): Promise<void> {
  const record = await sessionStore.load(sessionKey)
  if (!record) return
  const userContent: AcpxUserContent[] = [
    { Text: userMessageText } as AcpxUserContent,
  ]
  for (const _image of imageAttachments) {
    userContent.push({ Image: { source: 'base64' } } as AcpxUserContent)
  }
  const turnId = randomUUID()
  const updated = {
    ...record,
    messages: [
      ...record.messages,
      { User: { id: `user-${turnId}`, content: userContent } },
      { Agent: { content: [{ Text: assistantText }], tool_results: {} } },
    ],
    lastUsedAt: new Date().toISOString(),
  } as AcpSessionRecord
  await sessionStore.save(updated)
}

function recordToOpenAIMessages(record: AcpSessionRecord): OpenAIChatMessage[] {
  const messages: OpenAIChatMessage[] = []
  for (const message of record.messages) {
    if (message === 'Resume') continue
    if ('User' in message) {
      const text = message.User.content
        .map(userContentToText)
        .filter(Boolean)
        .join('\n\n')
        .trim()
      if (text) messages.push({ role: 'user', content: text })
      continue
    }
    if ('Agent' in message) {
      const text = message.Agent.content
        .map((part) => ('Text' in part ? part.Text : ''))
        .join('')
        .trim()
      if (text) messages.push({ role: 'assistant', content: text })
    }
  }
  return messages
}

type AcpxSessionMessage = AcpSessionRecord['messages'][number]
type AcpxUserContent = Extract<
  Exclude<AcpxSessionMessage, 'Resume'>,
  { User: unknown }
>['User']['content'][number]

function userContentToText(content: AcpxUserContent): string {
  if ('Text' in content) return unwrapPromptText(content.Text)
  if ('Mention' in content) return content.Mention.content
  if ('Image' in content) return content.Image.source ? '[image]' : ''
  return ''
}

function unwrapPromptText(raw: string): string {
  const runtimeMatch = raw.match(
    /^<browseros_acpx_runtime\b[\s\S]*?<\/browseros_acpx_runtime>\n\n<user_request>\n([\s\S]*?)\n<\/user_request>$/,
  )
  if (runtimeMatch) return decodeBasicEntities(runtimeMatch[1]).trim()
  const roleMatch = raw.match(
    /^<role>[\s\S]*?<\/role>\n\n<user_request>\n([\s\S]*?)\n<\/user_request>$/,
  )
  if (roleMatch) return decodeBasicEntities(roleMatch[1]).trim()
  return raw.trim()
}

function decodeBasicEntities(value: string): string {
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
}
