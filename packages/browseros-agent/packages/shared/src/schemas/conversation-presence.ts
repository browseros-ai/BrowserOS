/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { z } from 'zod'

export const ConversationRunStatusSchema = z.enum([
  'running',
  'completed',
  'aborted',
  'failed',
])

export const ConversationPresenceRunSchema = z.object({
  conversationId: z.string(),
  runId: z.string(),
  status: ConversationRunStatusSchema,
  tabIds: z.array(z.number().int()),
  updatedAt: z.number(),
})

export const TabConversationPresenceSchema = z.object({
  tabId: z.number().int(),
  conversationId: z.string(),
  runId: z.string(),
  status: ConversationRunStatusSchema,
  updatedAt: z.number(),
})

export const ConversationPresenceEventSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('snapshot'),
    runs: z.array(ConversationPresenceRunSchema),
    tabs: z.array(TabConversationPresenceSchema),
  }),
  z.object({
    type: z.literal('run-started'),
    run: ConversationPresenceRunSchema,
  }),
  z.object({
    type: z.literal('tab-touched'),
    tab: TabConversationPresenceSchema,
  }),
  z.object({
    type: z.literal('run-finished'),
    run: ConversationPresenceRunSchema,
  }),
  z.object({
    type: z.literal('conversation-forgotten'),
    conversationId: z.string(),
    tabIds: z.array(z.number().int()),
  }),
])

export type ConversationRunStatus = z.infer<typeof ConversationRunStatusSchema>
export type ConversationPresenceRun = z.infer<
  typeof ConversationPresenceRunSchema
>
export type TabConversationPresence = z.infer<
  typeof TabConversationPresenceSchema
>
export type ConversationPresenceEvent = z.infer<
  typeof ConversationPresenceEventSchema
>
