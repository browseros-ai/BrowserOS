import { registerDiagnostics } from '@browseros/diagnostics/extension'
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { defineBackground } from 'wxt/utils/define-background'
import { resolveBrowserOSServerBaseUrl } from '@/modules/api/browseros-ports'
import { createRecordingsRelay } from '@/modules/recorder'
import type { TakeoverResolveMessage } from '@/modules/takeover/takeover.types'
import { createTakeoverBridge } from '@/modules/takeover/takeover-bridge'

/** Supplies Chrome's trusted tab/document identity to the durable recorder relay. */
export default defineBackground(() => {
  registerDiagnostics('browseros-neo', resolveBrowserOSServerBaseUrl)
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
  const requestStop = (tabId: number) => {
    // The tab's session has ended; stop recording it through the cleanup grace.
    try {
      void chrome.tabs
        .sendMessage(tabId, { type: 'recorder-stop' })
        .catch(() => {})
    } catch {}
  }

  relay.onTabRecoveredAfterLoss(requestResnapshot)
  relay.onTabRecordingRetired(requestStop)
  void relay.start().catch((error) => {
    console.warn('[browseros-claw replay] durable outbox startup failed', error)
  })

  // Answers the in-page takeover bar: whether the sender's tab is the one a
  // parked agent is blocked on, and relays the human's Hand back.
  const takeover = createTakeoverBridge({
    resolveServerBaseUrl: resolveBrowserOSServerBaseUrl,
  })
  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    const takeoverMessage = message as { type?: unknown }
    if (takeoverMessage.type === 'takeover-poll') {
      const tabId = sender.tab?.id
      if (typeof tabId !== 'number') {
        sendResponse({ active: false })
        return false
      }
      void takeover
        .pollForTab(tabId)
        .then(sendResponse)
        .catch(() => sendResponse({ active: false }))
      return true
    }
    if (takeoverMessage.type === 'takeover-resolve') {
      const { sessionId, note } = message as TakeoverResolveMessage
      void takeover
        .resolve(sessionId, note)
        .then(() => sendResponse({ ok: true }))
        .catch(() => sendResponse({ ok: false }))
      return true
    }
    return false
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
