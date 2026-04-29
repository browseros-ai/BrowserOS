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
      // The legacy `resolveAgentSession` helper read from JSONL files
      // and went away with `OpenClawJsonlReader` (Step 11). The legacy
      // queue path is itself slated for deletion in Step 12; until then,
      // queued sends fall back to a fresh session key per message.
      resolveExistingSessionKey: () => null,
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
