/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { defineBackground } from 'wxt/utils/define-background'
import { resolveBrowserOSServerBaseUrl } from '@/modules/api/browseros-ports'
import { createRecordingsRelay } from '@/modules/recorder'

/** Supplies Chrome's trusted tab/document identity to the durable recorder relay. */
export default defineBackground(() => {
  const relay = createRecordingsRelay({
    resolveServerBaseUrl: resolveBrowserOSServerBaseUrl,
  })
  const requestResnapshot = (tabId: number) => {
    // Tabs can disappear between recovery detection and message delivery.
    try {
      void chrome.tabs
        .sendMessage(tabId, { type: 'recorder-resnapshot' })
        .catch(() => {})
    } catch {}
  }

  relay.onTabRecoveredAfterLoss(requestResnapshot)
  void relay.start().catch((error) => {
    console.warn('[browseros-claw replay] durable outbox startup failed', error)
  })

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    const recorderMessage = message as {
      type?: unknown
      ndjson?: unknown
      hasGap?: unknown
    }
    const tabId = sender.tab?.id
    const documentId = sender.documentId
    if (
      recorderMessage.type !== 'recorder-events' ||
      typeof recorderMessage.ndjson !== 'string' ||
      typeof recorderMessage.hasGap !== 'boolean' ||
      typeof tabId !== 'number' ||
      typeof documentId !== 'string'
    ) {
      return false
    }
    void relay
      .post(tabId, documentId, recorderMessage.ndjson, recorderMessage.hasGap)
      .then(() => sendResponse({ persisted: true }))
      .catch(() => sendResponse({ persisted: false }))
    return true
  })
})
