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

/**
 * Maps one browser tab's side panel to the conversation run it should display.
 * The server owns this assignment so clients never infer it from tool output.
 */
export const ConversationPanelAssignmentSchema = z.object({
  tabId: z.number().int(),
  conversationId: z.string(),
  runId: z.string(),
  status: ConversationRunStatusSchema,
})

/**
 * The server's complete current panel routing table. Every SSE message carries
 * all assignments, so a reconnect or heartbeat can repair missed client work
 * without replaying an event history.
 */
export const ConversationPanelAssignmentsSchema = z.object({
  assignments: z.array(ConversationPanelAssignmentSchema),
})

export type ConversationRunStatus = z.infer<typeof ConversationRunStatusSchema>
export type ConversationPanelAssignment = z.infer<
  typeof ConversationPanelAssignmentSchema
>
export type ConversationPanelAssignments = z.infer<
  typeof ConversationPanelAssignmentsSchema
>
