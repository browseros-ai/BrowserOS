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

export const ConversationPanelSchema = z.object({
  tabId: z.number().int(),
  conversationId: z.string(),
  runId: z.string(),
  status: ConversationRunStatusSchema,
})

/**
 * The server's complete tab-to-conversation projection. Every SSE message has
 * this same shape, so reconnect and live reconciliation follow one code path.
 */
export const ConversationPanelSnapshotSchema = z.object({
  tabs: z.array(ConversationPanelSchema),
})

export type ConversationRunStatus = z.infer<typeof ConversationRunStatusSchema>
export type ConversationPanel = z.infer<typeof ConversationPanelSchema>
export type ConversationPanelSnapshot = z.infer<
  typeof ConversationPanelSnapshotSchema
>
