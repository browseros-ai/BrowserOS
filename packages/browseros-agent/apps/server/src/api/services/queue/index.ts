/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { getOpenClawService } from '../openclaw/openclaw-service'
import { OutboundQueueService } from './outbound-queue-service'

let service: OutboundQueueService | null = null

/**
 * Lazy singleton — built on first access so the OpenClaw service is
 * already available. The queue subscribes to ClawSession state changes
 * via OpenClawService.onAgentStatusChange and dispatches through
 * OpenClawService.chatStream, so no extra wiring on the openclaw side.
 */
export function getOutboundQueueService(): OutboundQueueService {
  if (!service) {
    const openclaw = getOpenClawService()
    service = new OutboundQueueService({
      onAgentStatusChange: (listener) => openclaw.onAgentStatusChange(listener),
      getAgentState: (agentId) => openclaw.getAgentState(agentId),
      // Resolve the agent's existing user-chat session for queued sends
      // so we don't accidentally orphan the conversation by spawning a
      // fresh session per queued message. Only the very first message
      // for an agent (no prior session at all) falls back to a new key,
      // which mirrors what the existing /chat route does.
      resolveExistingSessionKey: (agentId) =>
        openclaw.resolveAgentSession(agentId).sessionKey ?? null,
      chatStream: ({
        agentId,
        sessionKey,
        message,
        history,
        messageParts,
        signal,
      }) =>
        openclaw.chatStream(agentId, sessionKey, message, history, {
          messageParts,
          signal,
        }),
    })
  }
  return service
}

/** Tear down the singleton — wired into server shutdown. */
export function shutdownOutboundQueueService(): void {
  if (service) {
    service.shutdown()
    service = null
  }
}

export type {
  QueuedItem,
  QueuedItemAttachmentPreview,
  QueuedItemPublic,
  QueuedItemStatus,
} from './outbound-queue-service'
