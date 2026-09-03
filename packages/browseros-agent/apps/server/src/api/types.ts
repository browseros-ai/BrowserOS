/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import {
  AcpAgentTargetSchema,
  BrowserOsAgentTargetSchema,
} from '@browseros/shared/schemas/agent'
import {
  type BrowserContext,
  BrowserContextSchema,
} from '@browseros/shared/schemas/browser-context'
import { LLMConfigSchema } from '@browseros/shared/schemas/llm'
import { z } from 'zod'
import type { ServerActivity } from './services/server-activity'

export type { BrowserContext }

export const AgentLLMConfigSchema = LLMConfigSchema.extend({
  model: z.string().min(1, 'Model name is required'),
  upstreamProvider: z.string().optional(),
})

const PreviousConversationSchema = z
  .union([
    z.array(
      z.object({
        role: z.enum(['user', 'assistant']),
        content: z.string(),
      }),
    ),
    z.string(),
  ])
  .optional()
  .transform((value) => {
    if (typeof value !== 'string') return value
    if (!value.trim()) return undefined
    return [{ role: 'user' as const, content: value }]
  })

const ChatInputSchema = z.object({
  conversationId: z.string().uuid(),
  message: z.string().optional().default(''),
  contextWindowSize: z.number().optional(),
  browserContext: BrowserContextSchema.optional(),
  userSystemPrompt: z.string().optional(),
  isScheduledTask: z.boolean().optional().default(false),
  userWorkingDir: z.string().min(1).optional(),
  supportsImages: z.boolean().optional().default(true),
  supportsReasoning: z.boolean().optional().default(true),
  mode: z.enum(['chat', 'agent']).optional().default('agent'),
  origin: z.enum(['sidepanel', 'newtab']).optional().default('sidepanel'),
  declinedApps: z.array(z.string()).optional(),
  selectedText: z.string().optional(),
  selectedTextSource: z
    .object({
      url: z.string(),
      title: z.string(),
    })
    .optional(),
  previousConversation: PreviousConversationSchema,
  // 'local': the server owns history in SQLite (load + persist). 'cloud': the
  // client owns history (logged-in cloud sync or incognito); the server stays
  // stateless and persists nothing. Defaults to 'cloud' so existing clients are
  // unchanged until they opt into server-owned history.
  historyMode: z.enum(['local', 'cloud']).optional().default('cloud'),
  attachments: z
    .array(
      z.object({
        mediaType: z.string().min(1),
        data: z.string().min(1),
      }),
    )
    .optional(),
})

/**
 * The provider half of a chat request, now optional.
 *
 * The server holds the provider list and which one is selected, so a client
 * only has to name an id, and need not even do that: with nothing given the
 * selected provider is used. Every field stays accepted because the extension
 * updates independently of the browser binary, so a shipped build can be
 * running a client that still sends the whole configuration inline. The server
 * can stop requiring these; it cannot stop accepting them.
 */
const OptionalAgentLLMConfigSchema = LLMConfigSchema.partial().extend({
  model: z.string().min(1).optional(),
  upstreamProvider: z.string().optional(),
})

const BrowserOsChatRequestSchema = OptionalAgentLLMConfigSchema.merge(
  ChatInputSchema,
)
  .extend({
    target: BrowserOsAgentTargetSchema.partial({ providerId: true }).optional(),
  })
  .transform((request) => ({
    ...request,
    target: {
      type: 'browseros' as const,
      providerId:
        request.target?.providerId || request.providerId || request.provider,
    },
  }))

const AcpChatRequestSchema = ChatInputSchema.extend({
  target: AcpAgentTargetSchema,
})

export const ChatRequestSchema = z.union([
  AcpChatRequestSchema,
  BrowserOsChatRequestSchema,
])

export type AcpChatRequest = z.infer<typeof AcpChatRequestSchema>
export type BrowserOsChatRequest = z.infer<typeof BrowserOsChatRequestSchema>

/**
 * A browseros request after its provider has been filled from the stored row.
 *
 * The wire shape leaves the provider optional so a client can send an id alone,
 * or nothing at all. Everything past the route boundary needs it resolved, and
 * this type is what says so rather than an assertion at the call site.
 */
export type HydratedBrowserOsChatRequest = BrowserOsChatRequest & {
  provider: NonNullable<BrowserOsChatRequest['provider']>
  target: { type: 'browseros'; providerId: string }
}

/** What the chat service works on: every provider already resolved. */
export type HydratedChatRequest = AcpChatRequest | HydratedBrowserOsChatRequest
export type ChatRequest = z.infer<typeof ChatRequestSchema>

export type Env = {
  Bindings: {
    server: ReturnType<typeof Bun.serve>
  }
}

export interface HttpServerConfig {
  port: number
  host?: string

  version: string
  browser: Browser
  browserSession: BrowserSession

  browserosId?: string
  executionDir: string
  resourcesDir: string
  aiSdkDevtoolsEnabled?: boolean
  activity?: ServerActivity
  onShutdown?: () => void
}
